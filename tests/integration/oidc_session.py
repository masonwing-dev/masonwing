"""Browser-grade OIDC session bootstrap for integration tests.

This performs the real Authorization Code + PKCE round trip that
``tests/web/e2e/runtime-auth.spec.ts`` performs with a real browser:

1. ``GET /auth/login`` on the BFF -> 307 redirect to Keycloak (state + PKCE
   challenge are stored server-side against the ``masonwing_session`` cookie).
2. ``GET`` the Keycloak auth URL -> real login form page.
3. ``POST`` the form action with the developer fixture credentials -> 302 back
   to ``/auth/callback`` with the one-time authorization code.
4. ``GET`` the callback URL -> BFF exchanges the code (PKCE verifier, state
   replay check) and marks the session authenticated.
5. ``GET /session`` -> session record + CSRF token.

Keycloak marks its session cookies ``Secure``/``SameSite=None``; on the
loopback-dev HTTP origin a real browser accepts them anyway, so the cookie jar
is configured to accept ``Secure`` cookies over ``http`` -- this mirrors the
browser behavior the flow is designed for and does not bypass authentication.

Secrets (OAuth codes, cookies, CSRF values) are never persisted to disk.
"""
from __future__ import annotations

import http.cookiejar
import json
import re
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from html import unescape

BFF_PUBLIC_ORIGIN = "http://localhost:39850"
USERNAME = "developer"
PASSWORD = "masonwing-local-developer"


@dataclass
class AuthenticatedSession:
    """Live session: opener carries cookies; headers carry CSRF + origin."""

    opener: urllib.request.OpenerDirector
    origin: str
    csrf_token: str
    principal_id: str
    tenant_ids: list[str] = field(default_factory=list)

    def headers(self, **extra: str) -> dict[str, str]:
        return {
            "Origin": self.origin,
            "X-CSRF-Token": self.csrf_token,
            "Content-Type": "application/json",
            **extra,
        }

    def request(
        self,
        url: str,
        body: object = None,
        method: str | None = None,
        headers: dict[str, str] | None = None,
        *,
        raw: bytes | None = None,
    ):
        """HTTP request on the authenticated session, returning (status, json, headers)."""
        data = raw if raw is not None else (None if body is None else json.dumps(body).encode())
        request = urllib.request.Request(
            url,
            data=data,
            headers={"Content-Type": "application/json", **(headers or {})},
            method=method,
        )
        try:
            response = self.opener.open(request, timeout=10)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            payload = response.read()
            return response.status, (json.loads(payload) if payload else None), dict(response.headers)


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):  # noqa: D102 - suppress auto-follow
        return None


def _open_no_redirect(opener: urllib.request.OpenerDirector, request: urllib.request.Request):
    try:
        return opener.open(request, timeout=10)
    except urllib.error.HTTPError as error:
        return error


def begin_login(
    origin: str = BFF_PUBLIC_ORIGIN,
    username: str = USERNAME,
    password: str = PASSWORD,
    step_up: bool = False,
) -> AuthenticatedSession:
    """Drive the real OIDC login against the running dev stack and return a session.

    If step_up is True, perform OIDC step-up authentication immediately after login:
    GET /auth/login?step_up=true -> Keycloak with higher ACR values -> POST credentials
    -> callback -> session has step_up_expires_at set.
    """
    # Accept `Secure` cookies over plain HTTP: loopback dev serves HTTP only, and
    # browsers treat localhost as a trustworthy origin. Without this, Python's
    # cookiejar silently drops Keycloak's AUTH_SESSION_ID/KC_RESTART cookies and
    # the login post fails with `cookie_not_found` -- a transport quirk, not an
    # auth assertion.
    policy = http.cookiejar.DefaultCookiePolicy(secure_protocols=("http", "https"))
    jar = http.cookiejar.CookieJar(policy)
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar), _NoRedirect)

    # 1. BFF login start -> 307 to Keycloak auth endpoint.
    login = _open_no_redirect(opener, urllib.request.Request(f"{origin}/auth/login"))
    assert login.status in (302, 307), f"/auth/login returned {login.status}"
    kc_auth_url = login.headers.get("Location")
    assert kc_auth_url and "openid-connect/auth" in kc_auth_url

    # 2. Keycloak login page -> extract form action (bound to session_code).
    page = _open_no_redirect(opener, urllib.request.Request(kc_auth_url))
    assert page.status == 200, f"Keycloak login page returned {page.status}"
    html = page.read().decode()
    match = re.search(r'<form[^>]*action="([^"]+)"', html)
    assert match, "Keycloak login form action not found"
    action_url = unescape(match.group(1))

    # 3. Submit credentials -> 302 back to BFF callback with auth code.
    body = urllib.parse.urlencode(
        {"username": username, "password": password, "credentialId": ""}
    ).encode()
    post = _open_no_redirect(
        opener,
        urllib.request.Request(
            action_url,
            data=body,
            headers={
                "Content-Type": "application/x-www-form-urlencoded",
                "Referer": kc_auth_url,
                "Origin": "http://localhost:39853",
            },
        ),
    )
    assert post.status in (302, 303), f"credential post returned {post.status}"
    callback_url = post.headers.get("Location")
    assert callback_url and "/auth/callback" in callback_url

    # 4. Callback -> BFF completes the exchange and marks session authenticated.
    callback = _open_no_redirect(opener, urllib.request.Request(callback_url))
    assert callback.status in (302, 303, 307), f"/auth/callback returned {callback.status}"

    # 5. Session record -> authenticated principal + CSRF token.
    session = _open_no_redirect(opener, urllib.request.Request(f"{origin}/session"))
    assert session.status == 200, f"/session returned {session.status}"
    data = json.loads(session.read())
    assert data.get("authenticated") is True, data
    csrf = data.get("csrf_token")
    principal = data.get("principal", {}).get("id")
    assert isinstance(csrf, str) and len(csrf) >= 32
    assert isinstance(principal, str)

    # 6. If step_up is required, perform the step-up flow
    if step_up:
        _perform_step_up_flow(opener, origin, username, password)
        # Re-fetch session to get updated step_up_expires_at
        session = _open_no_redirect(opener, urllib.request.Request(f"{origin}/session"))
        assert session.status == 200, f"/session after step-up returned {session.status}"
        data = json.loads(session.read())
        csrf = data.get("csrf_token")
        principal = data.get("principal", {}).get("id")

    return AuthenticatedSession(
        opener=opener,
        origin=origin,
        csrf_token=csrf,
        principal_id=principal,
        tenant_ids=list(data.get("tenant_ids") or []),
    )


def _perform_step_up_flow(
    opener: urllib.request.OpenerDirector,
    origin: str,
    username: str,
    password: str,
) -> None:
    """Perform the OIDC step-up flow: GET /auth/login?step_up=true -> Keycloak -> POST -> callback.

    This sets step_up_expires_at on the session, satisfying the LoA-2 requirement.
    """
    # 1. BFF step-up login start -> 307 to Keycloak auth endpoint with higher ACR.
    login = _open_no_redirect(
        opener,
        urllib.request.Request(f"{origin}/auth/login?step_up=true")
    )
    assert login.status in (302, 307), f"/auth/login?step_up=true returned {login.status}"
    kc_auth_url = login.headers.get("Location")
    assert kc_auth_url and "openid-connect/auth" in kc_auth_url

    # 2. Keycloak login page -> extract form action.
    page = _open_no_redirect(opener, urllib.request.Request(kc_auth_url))
    assert page.status == 200, f"Keycloak step-up page returned {page.status}"
    html = page.read().decode()
    match = re.search(r'<form[^>]*action="([^"]+)"', html)
    assert match, "Keycloak step-up form action not found"
    action_url = unescape(match.group(1))

    # 3. Submit credentials -> 302 back to BFF callback with step_up=true.
    body = urllib.parse.urlencode(
        {"username": username, "password": password, "credentialId": ""}
    ).encode()
    post = _open_no_redirect(
        opener,
        urllib.request.Request(
            action_url,
            data=body,
            headers={
                "Content-Type": "application/x-www-form-urlencoded",
                "Referer": kc_auth_url,
                "Origin": "http://localhost:39853",
            },
        ),
    )
    assert post.status in (302, 303), f"step-up credential post returned {post.status}"
    callback_url = post.headers.get("Location")
    assert callback_url and "/auth/callback" in callback_url

    # 4. Callback -> BFF completes step-up verification and sets step_up_expires_at.
    callback = _open_no_redirect(opener, urllib.request.Request(callback_url))
    assert callback.status in (302, 303, 307), f"/auth/callback step-up returned {callback.status}"

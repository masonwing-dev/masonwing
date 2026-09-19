import { useEffect, useMemo, useRef, useState } from 'react';
import { Link, useLocation, useNavigate } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Blocks, ChevronRight, Layers3, LogOut, Menu, Moon, RefreshCw, ShieldCheck, Sun, UserRound } from 'lucide-react';
import { ApiFailure, getDevStatus, getSession, logoutSession, type SessionView } from '@masonwing/api-client';
import { Badge, Button, Card, DataSurface, NavigationSheet } from '@masonwing/ui';
import { ContributionRegistry } from '@masonwing/plugin-sdk';
import { ProductScreen } from './ProductScreen';
import { clearAuthenticatedScope, clearPreviousTenant } from './tenant-scope';
import screens from './generated/screens.json';

const registry = new ContributionRegistry();
screens.forEach(screen => registry.register({
  id: screen.qualified_id,
  plugin: `${screen.product.toUpperCase()}@1.0.1`,
  route: screen.route,
  title: screen.title,
  feature: screen.feature_id,
  commands: screen.actions,
}));

function matchScreen(pathname: string) {
  return screens.find(screen => {
    const pattern = `^${screen.route.replace(':tenantId', '[^/]+').replace(':brandId', '[^/]+')}/?$`;
    return new RegExp(pattern).test(pathname);
  });
}

function tenantFromPath(pathname: string) {
  const match = pathname.match(/^\/workspace\/([^/]+)\//);
  return match ? decodeURIComponent(match[1]) : null;
}

function routeFor(route: string, tenantId: string) {
  return route.replace(':tenantId', encodeURIComponent(tenantId)).replace(':brandId', 'default');
}

function principalKey(session: SessionView | undefined) {
  return session?.principal ? `${session.principal.type}:${session.principal.issuer}:${session.principal.id}` : '';
}

export function App() {
  const location = useLocation();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [drawerOpen, setDrawerOpen] = useState(false);
  const menuTrigger = useRef<HTMLButtonElement>(null);
  const menuOpenedAt = useRef(location.pathname);
  const [theme, setTheme] = useState(() => window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
  const [epoch, setEpoch] = useState(0);
  const [preferredTenant, setPreferredTenant] = useState<string | null>(null);
  const priorPrincipal = useRef('');
  const session = useQuery({ queryKey: ['session'], queryFn: ({ signal }) => getSession(signal), staleTime: 30_000, retry: false });
  const screen = matchScreen(location.pathname);
  const isHome = location.pathname === '/';
  const requestedTenant = tenantFromPath(location.pathname);
  const authenticated = Boolean(session.data?.authenticated && session.data.principal);
  const selectedTenant = useMemo(() => {
    const tenants = session.data?.tenant_ids ?? [];
    if (preferredTenant && tenants.includes(preferredTenant)) return preferredTenant;
    if (requestedTenant && tenants.includes(requestedTenant)) return requestedTenant;
    if (session.data?.active_tenant_id && tenants.includes(session.data.active_tenant_id)) return session.data.active_tenant_id;
    return tenants[0] ?? null;
  }, [preferredTenant, requestedTenant, session.data]);
  const principal = principalKey(session.data);

  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  useEffect(() => {
    setDrawerOpen(false);
    document.title = `${screen?.title ?? (isHome ? 'Workspace' : 'Không tìm thấy')} · Masonwing`;
    document.querySelector<HTMLElement>('main')?.focus({ preventScroll: true });
    window.scrollTo(0, 0);
  }, [location.pathname, screen, isHome]);
  useEffect(() => {
    if (!principal) { priorPrincipal.current = ''; return; }
    if (priorPrincipal.current && priorPrincipal.current !== principal) {
      setPreferredTenant(null);
      void clearAuthenticatedScope(queryClient).then(() => setEpoch(value => value + 1));
    }
    priorPrincipal.current = principal;
  }, [principal, queryClient]);

  const logout = useMutation({
    mutationFn: () => logoutSession(session.data?.csrf_token ?? null),
    onSuccess: async () => {
      await clearAuthenticatedScope(queryClient);
      queryClient.setQueryData<SessionView>(['session'], {
        authenticated: false, principal: null, tenant_ids: [], active_tenant_id: null,
        expires_at: null, step_up_expires_at: null, csrf_token: null,
      });
      setEpoch(value => value + 1);
      setPreferredTenant(null);
      navigate('/', { replace: true });
    },
  });

  const switchTenant = async (nextTenant: string) => {
    if (!session.data?.tenant_ids.includes(nextTenant) || nextTenant === selectedTenant) return;
    const nextEpoch = epoch + 1;
    await clearPreviousTenant(queryClient, nextTenant, nextEpoch);
    setEpoch(nextEpoch);
    setPreferredTenant(nextTenant);
    const destination = screen ? routeFor(screen.route, nextTenant) : '/';
    navigate(destination, { replace: true });
  };

  useEffect(() => {
    if (!preferredTenant || !screen || !requestedTenant || requestedTenant === preferredTenant) return;
    navigate(routeFor(screen.route, preferredTenant), { replace: true });
  }, [navigate, preferredTenant, requestedTenant, screen]);

  const navigation = <>
    <Link to="/" className={`nav-home ${isHome ? 'active' : ''}`}><Layers3 size={18} aria-hidden="true" />Tổng quan</Link>
    <p className="nav-label">Platform</p>
    <nav aria-label="Mô-đun Masonwing">
      {screens.map(definition => selectedTenant ? <Link key={definition.qualified_id}
        to={routeFor(definition.route, selectedTenant)} className={`nav-link ${screen?.qualified_id === definition.qualified_id ? 'active' : ''}`}
        aria-current={screen?.qualified_id === definition.qualified_id ? 'page' : undefined}>
        <span className="feature-number">{definition.feature_id.slice(2)}</span><span>{definition.title.split(' — ')[0]}</span>
      </Link> : <span className="nav-link disabled" key={definition.qualified_id}><span className="feature-number">{definition.feature_id.slice(2)}</span><span>{definition.title.split(' — ')[0]}</span></span>)}
    </nav>
  </>;

  const requestedTenantForbidden = authenticated && requestedTenant !== null && !session.data!.tenant_ids.includes(requestedTenant);

  return <div className="app-shell">
    <a href="#main" className="skip-link">Đến nội dung chính</a>
    <aside className="sidebar">
      <Link to="/" className="wordmark"><span className="brand-icon"><Blocks size={22} aria-hidden="true" /></span>Masonwing<span className="brand-dot">.</span></Link>
      <div className="nav-scroll">{navigation}</div>
      <div className="sidebar-footer"><Badge>LOCAL</Badge><p>Contract 1.0.0</p></div>
    </aside>
    <div className="main-column">
      <header className="topbar">
        <Button ref={menuTrigger} className="mobile-menu" variant="ghost" aria-label="Mở điều hướng" onClick={() => { menuOpenedAt.current = location.pathname; setDrawerOpen(true); }}><Menu size={20} aria-hidden="true" /></Button>
        <div className="breadcrumb"><span>Masonwing</span><ChevronRight size={14} aria-hidden="true" /><strong>{screen ? screen.title.split(' — ')[0] : 'Workspace'}</strong></div>
        <div className="header-actions">
          {authenticated && selectedTenant ? <label className="tenant-switch"><span className="sr-only">Workspace</span><select aria-label="Workspace" value={selectedTenant} onChange={event => void switchTenant(event.target.value)}>
            {session.data!.tenant_ids.map(tenant => <option key={tenant} value={tenant}>{tenant}</option>)}
          </select></label> : null}
          {authenticated && session.data?.principal ? <span className="principal-chip" title={`${session.data.principal.type} · ${session.data.principal.issuer}`}><UserRound size={16} aria-hidden="true" />{session.data.principal.id}</span> : null}
          <Button variant="ghost" aria-label={theme === 'light' ? 'Bật giao diện tối' : 'Bật giao diện sáng'} onClick={() => setTheme(value => value === 'light' ? 'dark' : 'light')}>
            {theme === 'light' ? <Moon size={19} aria-hidden="true" /> : <Sun size={19} aria-hidden="true" />}
          </Button>
          {authenticated ? <Button variant="ghost" aria-label="Đăng xuất" disabled={logout.isPending} onClick={() => logout.mutate()}><LogOut size={18} aria-hidden="true" /></Button> : null}
        </div>
      </header>
      <main id="main" tabIndex={-1}>
        <div className="page-heading"><p className="eyebrow">{screen ? screen.feature_id : 'MASONWING'}</p>
          <h1>{screen?.title ?? (isHome ? 'Workspace' : 'Không tìm thấy trang')}</h1>
          <p>{screen ? 'Dữ liệu và thao tác dưới đây dùng trực tiếp same-origin BFF contract của tenant hiện hành.' : isHome ? 'Theo dõi phiên đăng nhập, workspace và capability thực tế của local platform.' : 'Đường dẫn này không nằm trong screen inventory hiện hành.'}</p>
        </div>

        {session.isPending ? <Card><DataSurface state="loading" /></Card>
          : session.isError ? <SessionFailure error={session.error} retry={() => void session.refetch()} returnTo={location.pathname + location.search} />
          : !authenticated ? <SignInPanel returnTo={location.pathname + location.search} />
          : requestedTenantForbidden ? <Card><DataSurface state="forbidden" /><p className="small-note">Tenant trong URL không thuộc session hiện hành. Chọn workspace được cấp quyền từ header.</p></Card>
          : !selectedTenant ? <Card><DataSurface state="empty" /><p className="small-note">Session hợp lệ nhưng chưa có tenant nào được cấp quyền.</p></Card>
          : isHome ? <Dashboard session={session.data!} tenantId={selectedTenant} />
          : screen ? <ProductScreen key={`${selectedTenant}:${screen.qualified_id}`} screen={screen} tenantId={selectedTenant} principalId={principal} epoch={epoch} csrf={session.data!.csrf_token ?? ''} />
          : <Button asChild><Link to="/">Về tổng quan</Link></Button>}

        {logout.isError ? <div className="global-error" role="alert">Không đăng xuất được. {logout.error instanceof Error ? logout.error.message : ''}</div> : null}
        <footer className="page-footer">Local environment · Same-origin session · External effects follow backend policy</footer>
      </main>
    </div>
    <NavigationSheet open={drawerOpen} onOpenChange={setDrawerOpen} onCloseAutoFocus={event => {
      event.preventDefault();
      if (menuOpenedAt.current === location.pathname) menuTrigger.current?.focus();
      else document.querySelector<HTMLElement>('main')?.focus({ preventScroll: true });
    }}>{navigation}</NavigationSheet>
  </div>;
}

function SignInPanel({ returnTo }: { returnTo: string }) {
  return <Card className="sign-in-panel"><span className="signin-icon"><ShieldCheck size={24} aria-hidden="true" /></span><div><h2>Đăng nhập để mở workspace</h2>
    <p>Authentication đi qua Rust BFF. Browser chỉ dùng HttpOnly session cookie và CSRF token của session view.</p>
    <Button asChild variant="primary"><a href={`/auth/login?return_to=${encodeURIComponent(returnTo)}`}>Đăng nhập</a></Button></div></Card>;
}

function SessionFailure({ error, retry, returnTo }: { error: unknown; retry: () => void; returnTo: string }) {
  const api = error instanceof ApiFailure ? error : null;
  if (api?.status === 401) return <SignInPanel returnTo={returnTo} />;
  return <Card><DataSurface state={error instanceof TypeError ? 'offline' : 'error'} retry={retry} />
    {api?.body?.correlation_id ? <p className="small-note">Correlation {api.body.correlation_id}</p> : null}</Card>;
}

function Dashboard({ session, tenantId }: { session: SessionView; tenantId: string }) {
  const status = useQuery({ queryKey: ['local-dev-status'], queryFn: ({ signal }) => getDevStatus(signal), retry: false, refetchInterval: 15_000 });
  return <div className="dashboard-stack">
    <Card className="welcome-card"><div><Badge tone="positive">SESSION ACTIVE</Badge><h2>{tenantId}</h2><p>Workspace đang dùng principal <strong>{session.principal?.id}</strong>. Chuyển tenant từ header sẽ hủy request cũ và xóa tenant cache trước khi tải dữ liệu mới.</p></div>
      <Button asChild variant="primary"><Link to={routeFor('/workspace/:tenantId/composition', tenantId)}>Mở composition<ChevronRight size={16} aria-hidden="true" /></Link></Button></Card>
    <div className="dashboard-grid">
      <Card><div className="section-row"><h2>Runtime</h2>{status.isFetching && !status.isPending ? <RefreshCw size={16} aria-hidden="true" /> : null}</div>
        {status.isPending ? <DataSurface state="loading" /> : status.isError ? <DataSurface state={status.error instanceof TypeError ? 'offline' : 'error'} retry={() => void status.refetch()} /> : <>
          <div className="status-pair"><span>Service</span><strong>{status.data.service}</strong></div>
          <div className="status-pair"><span>Environment</span><Badge>{status.data.environment}</Badge></div>
          <div className="status-pair"><span>External mutation</span><strong>{status.data.external_mutations_enabled ? 'Enabled' : 'Locked'}</strong></div>
          <div className="status-pair"><span>Live budget</span><strong>{status.data.live_budget_microunits}</strong></div>
        </>}
      </Card>
      <Card><h2>Session</h2>
        <div className="status-pair"><span>Principal type</span><strong>{session.principal?.type}</strong></div>
        <div className="status-pair"><span>Tenants</span><strong>{session.tenant_ids.length}</strong></div>
        <div className="status-pair"><span>Expires</span><strong>{formatSessionTime(session.expires_at)}</strong></div>
        <div className="status-pair"><span>Step-up</span><strong>{formatSessionTime(session.step_up_expires_at)}</strong></div>
      </Card>
    </div>
  </div>;
}

function formatSessionTime(value: string | null) {
  if (!value) return '—';
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : new Intl.DateTimeFormat('vi-VN', { dateStyle: 'short', timeStyle: 'short' }).format(date);
}

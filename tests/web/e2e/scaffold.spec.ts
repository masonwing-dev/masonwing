import { test, expect, type Page, type Route } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import screens from '../../../packages/web-shell/src/generated/screens.json' with { type: 'json' };

const session = {
  authenticated: true,
  principal: { type: 'USER', id: 'user-1', issuer: 'https://issuer.local' },
  tenant_ids: ['tenant-a', 'tenant-b'],
  active_tenant_id: 'tenant-a',
  expires_at: '2026-09-17T00:00:00Z',
  step_up_expires_at: null,
  csrf_token: 'csrf-1',
};

function fulfillJson(route: Route, body: unknown, status = 200) {
  return route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
}

async function installProductFixtures(page: Page) {
  await page.route('**/session', route => fulfillJson(route, session));
  await page.route('**/dev/status', route => fulfillJson(route, {
    service: 'masonwing-host-api', environment: 'LOCAL', external_mutations_enabled: false, live_budget_microunits: 0,
    readiness: { ready: false, status: 'QUALIFICATION_INCOMPLETE' }, operation_catalog_count: 43,
    data_store: 'POSTGRESQL', artifact_store: 'S3', identity: 'OIDC', authorization: 'CEDAR',
  }));
  await page.route('**/v1/tenants/*/resources/**', route => fulfillJson(route, {
    items: [], next_cursor: null, snapshot_at: '2026-09-16T10:00:00Z', data_state: 'NO_DATA', total_visible: 0,
  }));
  await page.route('**/v1/tenants/*/views/**', route => fulfillJson(route, {
    items: [], next_cursor: null, snapshot_at: '2026-09-16T10:00:00Z', data_state: 'NO_DATA', total_visible: 0,
  }));
}

for (const width of [320, 390, 768, 1440]) {
  for (const theme of ['light', 'dark'] as const) {
    test(`functional workspace ${width}px ${theme}`, async ({ page }, testInfo) => {
      await installProductFixtures(page);
      await page.setViewportSize({ width, height: 960 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto('/');
      await expect(page.getByRole('heading', { level: 1 })).toHaveText('Workspace');
      const workspace = page.getByRole('combobox', { name: 'Workspace' });
      await expect(workspace).toBeVisible();
      await expect(workspace).toHaveValue('tenant-a');
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
      const violations = (await new AxeBuilder({ page }).analyze()).violations.filter(item => ['serious', 'critical'].includes(item.impact ?? ''));
      expect(violations).toEqual([]);
      await page.screenshot({ path: testInfo.outputPath(`workspace-${width}-${theme}.png`), fullPage: true });

      if (width < 768) {
        const opener = page.getByRole('button', { name: 'Mở điều hướng' });
        await opener.click();
        await expect(page.getByRole('dialog')).toBeVisible();
        await page.keyboard.press('Escape');
        await expect(page.getByRole('dialog')).not.toBeVisible();
        await expect(opener).toBeFocused();
      }

      await page.goto('/workspace/tenant-a/composition');
      await expect(page.getByRole('heading', { level: 1 })).toHaveText('Lắp ghép sản phẩm, không fork kernel');
      await expect(page.getByText('product.compose', { exact: true })).toBeVisible();
      const emptyCollections = page.getByRole('heading', { name: 'Chưa có dữ liệu', exact: true });
      await expect(emptyCollections).toHaveCount(2); // Composition view and registry input collection.
      for (const empty of await emptyCollections.all()) await expect(empty).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
      await page.screenshot({ path: testInfo.outputPath(`composition-${width}-${theme}.png`), fullPage: true });
    });
  }
}

test('all 23 feature routes remain reachable at 320px with 200% text', async ({ page }, testInfo) => {
  test.setTimeout(120000);
  await installProductFixtures(page);
  await page.setViewportSize({ width: 320, height: 900 });
  for (const definition of screens) {
    await page.goto(definition.route.replace(':tenantId', 'tenant-a').replace(':brandId', 'default'));
    await page.addStyleTag({ content: ':root { font-size: 32px !important; }' });
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(definition.title);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    for (const operation of definition.actions) await expect(page.getByText(operation, { exact: true }).first()).toBeVisible();

    const serious = (await new AxeBuilder({ page }).analyze()).violations.filter(item => ['serious', 'critical'].includes(item.impact ?? ''));
    expect(serious, definition.qualified_id).toEqual([]);
    if (definition.feature_id === 'F-001') await page.screenshot({ path: testInfo.outputPath('composition-320-200-percent.png'), fullPage: true });
  }
});

test('login preserves a protected deep link including resource selection', async ({ page }) => {
  await page.route('**/session', route => fulfillJson(route, { ...session, authenticated: false, principal: null, tenant_ids: [], active_tenant_id: null, csrf_token: null }));
  const target = '/workspace/tenant-a/workflow?type=Run&resource=run-7';
  await page.goto(target);
  await expect(page.getByRole('link', { name: 'Đăng nhập' })).toHaveAttribute('href', `/auth/login?return_to=${encodeURIComponent(target)}`);
});

test('guided command sends CSRF and idempotency proof and keeps ACCEPTED pending', async ({ page }) => {
  await installProductFixtures(page);
  let commandHeaders: Record<string, string> | null = null;
  let commandBody: unknown;
  await page.route('**/v1/tenants/*/commands/budget.configure', route => {
    commandHeaders = route.request().headers();
    commandBody = route.request().postDataJSON();
    return fulfillJson(route, {
      command_id: 'cmd-budget-1', state: 'ACCEPTED', resource: null, run_id: null, effect_id: null,
      correlation_id: 'corr-budget-1', accepted_at: '2026-09-16T10:01:00Z',
    }, 202);
  });
  await page.goto('/workspace/tenant-a/budget');
  await page.getByLabel(/Currency/).fill('USD');
  await page.getByLabel(/Limit \(microunits\)/).fill('250000');
  await page.getByLabel(/Chu kỳ/).selectOption('DAY');
  await page.getByLabel(/Expected version/).fill('4');
  await page.getByRole('button', { name: 'Gửi thao tác' }).click();
  await expect(page.getByText('Command đã được nhận')).toBeVisible();
  await expect(page.getByText(/chưa phải business success/i)).toBeVisible();
  expect(commandHeaders?.['x-csrf-token']).toBe('csrf-1');
  expect(commandHeaders?.['idempotency-key']).toBeTruthy();
  expect(commandBody).toEqual({ currency: 'USD', limit_microunits: 250000, period: 'DAY', expected_version: 4 });
});

import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import screens from '../../../packages/web-shell/src/generated/screens.json' with { type: 'json' };

for (const width of [320, 390, 768, 1440]) {
  for (const theme of ['light', 'dark'] as const) {
    test(`development shell ${width}px ${theme}`, async ({ page }, testInfo) => {
      await page.setViewportSize({ width, height: 960 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto('/');
      await expect(page.getByRole('heading', { level: 1 })).toHaveText('Không gian phát triển');
      await expect(page.getByText('Rust API đang phản hồi')).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
      const violations = (await new AxeBuilder({ page }).analyze()).violations.filter(v => ['serious', 'critical'].includes(v.impact ?? ''));
      expect(violations).toEqual([]);
      await page.screenshot({ path: testInfo.outputPath(`home-${width}-${theme}.png`), fullPage: true });
      if (width < 768) {
        const opener = page.getByRole('button', { name: 'Mở điều hướng' });
        await opener.click();
        await expect(page.getByRole('dialog')).toBeVisible();
        await page.keyboard.press('Escape');
        await expect(page.getByRole('dialog')).not.toBeVisible();
        await expect(opener).toBeFocused();
      }
      await page.goto('/workspace/local-dev/composition');
      await expect(page.getByRole('heading', { level: 1 })).toHaveCount(1);
      await expect(page.getByRole('button', { name: 'product.compose' })).toBeDisabled();
      await page.locator('summary').first().click();
      await expect(page.locator('.oracle').first()).toBeVisible();
      await page.screenshot({ path: testInfo.outputPath(`composition-${width}-${theme}.png`), fullPage: true });
    });
  }
}

test('all 45 route scaffolds remain reachable at 320px with 200% text', async ({ page }, testInfo) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 320, height: 900 });
  for (const screen of screens) {
    await page.goto(screen.route.replace(':tenantId', 'local-dev').replace(':brandId', 'local-brand'));
    await page.addStyleTag({ content: ':root { font-size: 32px !important; }' });
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(screen.title);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    const headerOverlap = await page.evaluate(() => {
      const title = document.querySelector('.breadcrumb strong')!.getBoundingClientRect();
      const actions = document.querySelector('.header-actions')!.getBoundingClientRect();
      return title.left < actions.right && title.right > actions.left
        && title.top < actions.bottom && title.bottom > actions.top;
    });
    expect(headerOverlap, `${screen.qualified_id}: header label must not be covered by actions`).toBe(false);
    for (const command of screen.actions) await expect(page.getByRole('button', { name: command, exact: true })).toBeDisabled();
    if (screen.feature_id === 'F-001') {
      await page.locator('summary').first().waitFor();
      await page.screenshot({ path: testInfo.outputPath(`${screen.product}-200-percent.png`) });
    }
  }
});

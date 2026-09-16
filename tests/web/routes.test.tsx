import { expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { App } from '../../packages/web-shell/src/App';
import screens from '../../packages/web-shell/src/generated/screens.json';

it.each(screens)('$qualified_id retains one global heading and no enabled business command', definition => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<QueryClientProvider client={client}><MemoryRouter initialEntries={[definition.route.replace(':tenantId', 'local-dev').replace(':brandId', 'local-brand')]}><App /></MemoryRouter></QueryClientProvider>);
  expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
  expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(definition.title);
  for (const command of definition.actions) expect(screen.getByRole('button', { name: command })).toBeDisabled();
  expect(screen.getByRole('status')).toHaveTextContent('Handler nghiệp vụ chưa được triển khai');
  client.clear();
});

it('keeps unavailable API data distinguishable from zero business metrics', async () => {
  vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('offline')));
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<QueryClientProvider client={client}><MemoryRouter><App /></MemoryRouter></QueryClientProvider>);
  expect(await screen.findByRole('alert')).toHaveTextContent('Không tải được dữ liệu');
  expect(screen.queryByText('Rust API đang phản hồi')).not.toBeInTheDocument();
  client.clear();
});

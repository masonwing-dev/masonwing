import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { Link, useLocation } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { ArrowRight, Blocks, BookOpen, Box, Check, ChevronRight, Code2, Layers3, Menu, Moon, Sun, Terminal, Workflow } from 'lucide-react';
import { Badge, Button, Card, DataSurface, NavigationSheet } from '@masonwing/ui';
import { ContributionRegistry } from '@masonwing/plugin-sdk';
import { getDevStatus } from '@masonwing/api-client';
import screens from './generated/screens.json';
import summary from './generated/summary.json';
const AcceptanceCases = lazy(() => import('./AcceptanceCases'));

const registry = new ContributionRegistry();
screens.forEach(s => registry.register({ id: s.qualified_id, plugin: `${s.product.toUpperCase()}@1.0.1`,
  route: s.route, title: s.title, feature: s.feature_id, commands: s.actions }));
const totals = Object.values(summary).reduce((acc, s) => ({
  features: acc.features + s.features, requirements: acc.requirements + s.requirements,
  tests: acc.tests + s.test_cases, commands: acc.commands + s.commands,
}), { features: 0, requirements: 0, tests: 0, commands: 0 });

const localRoute = (route: string) => route.replace(':tenantId', 'local-dev').replace(':brandId', 'local-brand');

const services = [
  ['Web shell', 39850, 'React · Vite', '/'],
  ['Rust BFF', 39851, 'Axum · local status', 'http://localhost:39851/dev/status'],
  ['PostgreSQL', 39852, 'Business data · RLS', null],
  ['Keycloak', 39853, 'Identity · OIDC', 'http://localhost:39853'],
  ['Temporal', 39854, 'Durable engine · gRPC', null],
  ['Temporal UI', 39855, 'Workflow history', 'http://localhost:39855'],
  ['Object storage', 39856, 'S3 API', null],
  ['Storage console', 39857, 'Local artifact browser', 'http://localhost:39857'],
  ['OpenBao', 39858, 'Scoped secret storage', 'http://localhost:39858/ui'],
  ['Provider fixtures', 39859, 'Synthetic fault scenarios', 'http://localhost:39859/health'],
] as const;

export function App() {
  const location = useLocation();
  const [open, setOpen] = useState(false);
  const menuTrigger = useRef<HTMLButtonElement>(null);
  const menuOpenedAt = useRef(location.pathname);
  const [product, setProduct] = useState('masonwing');
  const [theme, setTheme] = useState(() => window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
  const screen = screens.find(s => localRoute(s.route) === location.pathname);
  const isHome = location.pathname === '/';
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  useEffect(() => {
    setOpen(false);
    if (screen) setProduct(screen.product);
    document.title = `${screen?.title ?? (isHome ? 'Development' : 'Không tìm thấy')} · Masonwing`;
    document.querySelector<HTMLElement>('main')?.focus({ preventScroll: true });
    window.scrollTo(0, 0);
  }, [location.pathname]);

  const availableProducts = Array.from(new Set(screens.map(s => s.product)));

  const navigation = <>
    <Link to="/" className={`nav-home ${isHome ? 'active' : ''}`}><Layers3 size={18} aria-hidden="true" />Overview</Link>
    {availableProducts.length > 1 && (
      <div className="product-switch" role="group" aria-label="Sản phẩm">
        <button aria-pressed={product === 'masonwing'} onClick={() => setProduct('masonwing')}>Framework</button>
        <button aria-pressed={product === 'gleanbird'} onClick={() => setProduct('gleanbird')}>Gleanbird</button>
      </div>
    )}
    <p className="nav-label">{product === 'masonwing' ? 'Core Platform' : 'Marketing plugins'}</p>
    <nav aria-label="Mô-đun">
      {screens.filter(s => s.product === product).map(s => <Link key={s.qualified_id}
        to={localRoute(s.route)} className={`nav-link ${screen?.qualified_id === s.qualified_id ? 'active' : ''}`}
        aria-current={screen?.qualified_id === s.qualified_id ? 'page' : undefined}>
        <span className="feature-number">{s.feature_id.slice(2)}</span><span>{s.title.split(' — ')[0]}</span>
      </Link>)}
    </nav>
  </>;

  return <div className="app-shell">
    <a href="#main" className="skip-link">Đến nội dung chính</a>
    <aside className="sidebar">
      <Link to="/" className="wordmark"><span className="brand-icon"><Blocks size={22} aria-hidden="true" /></span>Masonwing<span className="brand-dot">.</span></Link>
      <div className="nav-scroll">{navigation}</div>
      <div className="sidebar-footer"><Badge>LOCAL DEVELOPMENT</Badge><p>Spec 1.0.1 · Contract 1.0.0</p></div>
    </aside>
    <div className="main-column">
      <header className="topbar">
        <Button ref={menuTrigger} className="mobile-menu" variant="ghost" aria-label="Mở điều hướng" onClick={() => { menuOpenedAt.current = location.pathname; setOpen(true); }}><Menu size={20} aria-hidden="true" /></Button>
        <div className="breadcrumb"><span>Workspace</span><ChevronRight size={14} aria-hidden="true" /><strong>{screen ? (screen.product === 'masonwing' ? 'Framework' : 'Gleanbird') : 'Development'}</strong></div>
        <div className="header-actions"><Badge tone="warning">SCAFFOLD</Badge><Button variant="ghost" aria-label={theme === 'light' ? 'Bật giao diện tối' : 'Bật giao diện sáng'} onClick={() => setTheme(theme === 'light' ? 'dark' : 'light')}>
          {theme === 'light' ? <Moon size={19} aria-hidden="true" /> : <Sun size={19} aria-hidden="true" />}
        </Button></div>
      </header>
      <main id="main" tabIndex={-1}>
        <div className="page-heading"><p className="eyebrow">{screen ? `${screen.product} / ${screen.feature_id}` : 'MASONWING DEVELOPER WORKSPACE'}</p>
          <h1>{screen?.title ?? (isHome ? 'Không gian phát triển' : 'Không tìm thấy trang')}</h1>
          <p>{screen ? 'Hợp đồng, hành vi kỳ vọng và điểm bắt đầu cho mô-đun này.' : isHome ? 'Một kernel. Các miền nghiệp vụ độc lập. Phát triển từ những kết quả đã được đặc tả.' : 'Đường dẫn này chưa được đăng ký trong workspace.'}</p>
        </div>
        {isHome ? <Dashboard /> : screen ? <>
          <Card><div className="section-row"><Badge>{screen.feature_id}</Badge><Badge tone="warning">HANDLER CHƯA TRIỂN KHAI</Badge></div>
            <DataSurface state="not-implemented" />
            <div className="command-list">{screen.actions.map(command => <Button key={command} disabled title="Cần triển khai và kiểm chứng handler"><Code2 size={16} aria-hidden="true" /><code>{command}</code></Button>)}</div>
          </Card>
          <Suspense fallback={<p className="small-note">Đang tải các kết quả kiểm thử kỳ vọng…</p>}>
            <AcceptanceCases key={screen.qualified_id} product={screen.product} feature={screen.feature_id} />
          </Suspense>
        </> : <Button asChild><Link to="/">Về tổng quan</Link></Button>}
        <footer className="page-footer">Local synthetic data · External mutations disabled · Product acceptance pending</footer>
      </main>
    </div>
    <NavigationSheet open={open} onOpenChange={setOpen} onCloseAutoFocus={event => {
      event.preventDefault();
      if (menuOpenedAt.current === location.pathname) menuTrigger.current?.focus();
      else document.querySelector<HTMLElement>('main')?.focus({ preventScroll: true });
    }}>{navigation}</NavigationSheet>
  </div>;
}

function Dashboard() {
  const status = useQuery({ queryKey: ['local-dev-status'], queryFn: ({ signal }) => getDevStatus(signal), retry: false, refetchInterval: 15000 });
  return <>
    <Card className="intro-card"><div><Badge tone="positive">CONTRACT-FIRST</Badge><h2>Bắt đầu từ một nền tảng rõ ràng.</h2>
      <p>Rust cho runtime, React cho workspace, các plugin giao tiếp qua SDK. Mỗi thay đổi đi cùng kết quả kỳ vọng và bằng chứng kiểm thử.</p>
      <Button asChild variant="primary"><Link to="/workspace/local-dev/composition">Khám phá framework<ArrowRight size={17} aria-hidden="true" /></Link></Button>
    </div><div className="architecture-sketch" aria-label="Cấu trúc hệ thống"><span><Box size={20} aria-hidden="true" />Domain plugins</span><span><Workflow size={20} aria-hidden="true" />Public SDK & contracts</span><span><Layers3 size={20} aria-hidden="true" />Masonwing kernel</span></div></Card>
    <div className="stat-grid">{[['Mô-đun đặc tả', totals.features], ['Yêu cầu nghiệp vụ', totals.requirements], ['Ca kiểm thử thiết kế', totals.tests], ['Command contracts', totals.commands]].map(([label, value]) =>
      <Card key={label}><p>{label}</p><strong>{value.toLocaleString('vi-VN')}</strong><span>Trong hai bộ requirements</span></Card>)}</div>
    <div className="dashboard-grid">
      <Card><div className="section-row"><h2>Trạng thái ứng dụng</h2><Terminal size={19} aria-hidden="true" /></div>
        {status.isPending ? <DataSurface state="loading" /> : status.isError ? <DataSurface state="error" retry={() => void status.refetch()} /> : <>
          <div className="runtime-line"><span className="check-icon"><Check size={16} aria-hidden="true" /></span><div><strong>Rust API đang phản hồi</strong><p className="muted">{status.data.service} · {status.data.environment}</p></div></div>
          <div className="status-pair"><span>Write readiness</span><Badge tone="warning">CHƯA SẴN SÀNG</Badge></div>
          <div className="status-pair"><span>Tác động bên ngoài</span><strong>Đã khóa</strong></div>
          <div className="status-pair"><span>Ngân sách live</span><strong>0</strong></div>
          <p className="small-note">Health của tiến trình không đồng nghĩa các adapter hoặc luồng nghiệp vụ đã được nghiệm thu.</p>
        </>}
      </Card>
      <Card><div className="section-row"><h2>Vòng lặp phát triển</h2><BookOpen size={19} aria-hidden="true" /></div>
        <div className="workflow-step"><span>01</span><div><strong>Đọc domain và hợp đồng</strong><p>Chọn feature, aggregate và acceptance case.</p></div></div>
        <div className="workflow-step"><span>02</span><div><strong>Chạy test RED</strong><p>Kiểm tra đầu ra, trạng thái và tác động bị từ chối.</p></div></div>
        <div className="workflow-step"><span>03</span><div><strong>Implement → kiểm chứng</strong><p>Làm test đạt rồi giữ bằng chứng thực tế.</p></div></div>
      </Card>
    </div>
    <section className="services-section"><div className="section-row"><div><h2>Local development stack</h2><p className="muted">Endpoint đã cấu hình · loopback 39850–39859. Trạng thái dịch vụ: <code>make dev-status</code>.</p></div></div>
      <div className="service-grid">{services.map(([name, port, detail, href]) => <Card key={name}><div className="section-row"><strong>{name}</strong><code>{port}</code></div><p>{detail}</p>
        {href ? <a href={href} className="service-link" target={href.startsWith('http') ? '_blank' : undefined} rel="noreferrer">Mở dịch vụ<ArrowRight size={15} aria-hidden="true" /></a> : <span className="protocol-label">Kết nối qua client</span>}
      </Card>)}</div>
    </section>
  </>;
}

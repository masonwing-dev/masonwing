import { useState } from 'react';
import { Badge } from '@masonwing/ui';
import criteria from './generated/criteria.json';

export default function AcceptanceCases({ product, feature }: { product: string; feature: string }) {
  const [search, setSearch] = useState('');
  const matches = criteria.filter(c => c.product === product && c.feature_id === feature)
    .filter(c => JSON.stringify(c).toLocaleLowerCase('vi').includes(search.toLocaleLowerCase('vi')));
  return <section className="acceptance-section" aria-label="Các kết quả kiểm thử kỳ vọng">
    <div className="section-row"><div><h2>Hành vi cần đạt</h2>
      <p className="muted">Given → When → Then từ spec gốc; chưa phải bằng chứng nghiệm thu.</p>
    </div></div>
    <label className="search-label">Tìm ca kiểm thử
      <input type="search" placeholder="Tên ca hoặc kết quả kỳ vọng…" value={search} onChange={e => setSearch(e.target.value)} />
    </label>
    <div className="criteria-list">{matches.map(c => <details className="criterion" key={c.qualified_id}>
      <summary><span><code>{c.qualified_id.split(':').at(-1)}</code><strong>{c.title}</strong></span><Badge tone="warning">RED</Badge></summary>
      <div className="criterion-body">
        <p><strong>Given</strong> {c.preconditions?.join(' · ')}</p>
        <p><strong>When</strong> {c.steps?.join(' → ')}</p>
        <div className="oracle"><strong>Then</strong>{c.expected?.map((e, i) => <p key={i}>{e}</p>)}</div>
        <code>{c.suite_id}</code>
      </div>
    </details>)}</div>
    {matches.length === 0 ? <p className="muted">Không có ca kiểm thử khớp bộ lọc này.</p> : null}
  </section>;
}

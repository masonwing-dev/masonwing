# Đối chiếu với ZIP chuẩn của người dùng

Toàn bộ actual files trong source ZIP được giữ nguyên ở vendor;chỉ bỏ AppleDouble metadata. Tracking resources là optional mode,chưa khởi tạo ledger theo yêu cầu create. Không có yêu cầu template nào bị hiểu thành permission implement/deploy product.

| Rule group | Disposition | Evidence location |
|---|---|---|
| SKILL modes/create | create only;no tracker/CI/product activation | 00-start/IMPLEMENTATION-ORDER.md |
| Read source/discovery | Inputs preserved+hashed;goal/scope/actors/constraints | inputs/;SRS |
| Features/EARS | One requirement obligation,one shall,<=45 words;dense/stable IDs | SRS;validator |
| AC every REQ | Given/When/Then with observable outcome and negative absence | SRS;traceability.csv |
| All18 probes | Each probe mapped to feature/requirements | PROBE-COVERAGE.md |
| ISO quality | 2023 edition,9 characteristics,measured procedure targets | SRS Quality Requirements |
| ADRs | Context/Decision/Consequences;only explicit user stack Accepted | SRS Decisions |
| Provenance | Every supplied threshold/priority tagged proposed with owner | assumptions.json |
| Open questions | Unresolved external authority blocks related activation;local design concrete | open-questions.json |
| Machine audit | Original validator unmodified,errors fixed,warnings disposition | 09-audit/ |
| Semantic audit | Author self-review distinct from independent approval | 09-audit/SEMANTIC-REVIEW.md |
| Tracking boundary | Full templates retained;not initialized by SRS request | TRACKING-BOUNDARY.md |
| Change control | Stable IDs,evidence freshness,do not carry PASS | CHANGE-CONTROL.md |
| Output naming/header | plans/reports/requirements-YYMMDD-HHMM-slug.md;Source/Exported | SRS |
| No product claims | NOT_RUN and pending release gates;spec checks not acceptance | release-gates.json |

## Source file inventory

| Path | SHA256 |
|---|---|
| SKILL.md | fdb1b68667239c066e9ed6e1bb4d7d7aed0598ef88e9c9d6d8d21605d4d49c0b |
| assets/spec-template.md | e4b15d94c081e1a6d8926ad8f937840fb02df16d9c5e96c1f755da19a113c672 |
| assets/tracking/change-request.md | ef346fd198c91a1e1c0baec5c63f69fecb3146727cbe8c225f45d79f3ffe5698 |
| assets/tracking/ledger-transition.json | d44bb05d6c91a3df32575b2a5636be16754cf4e81f0a83ace1153eb64190049b |
| assets/tracking/ledger.json | 66d9b65c0fcfab66277a7a3f68f68842228c4dca2a4ef7a9e9e721fce5962aa2 |
| assets/tracking/mapping.json | 0743cedceb9085dea751448c21fb8eb6c3c553eccad63ff441fa93e49c69f66c |
| assets/tracking/review.md | a7a9511dda885599e6972e303e0a652faacd0153517b51532feed9ea33833e58 |
| assets/tracking/run-evidence.json | ba8db3f22212cf01b1a36cf08b2460fa967ec6bb5cfe4f3f55aec21bca498f5e |
| assets/tracking/work-item.md | 00a5db3f00c3fa1ccbc892822ba62266a7dd53221e9c1b6a15253858f6c64348 |
| references/acceptance.md | 26a1b5e4621671b1f476d7a1c34b804b66224c8c29f31840ab07c9ffc873709f |
| references/adr.md | 22b1345e2f543be2a3db2dc0e1be852cd4edd0fcbdc38ec6c4c36356852a48f8 |
| references/audit.md | a6999d51299325df91853a26fb926fc0a737d5996b1d27058cdefe88d790713d |
| references/discovery.md | 04b0d1e61f7f7f6a54b1eecf03abe0dfd2067e661f14aaaf1ed5d0879d433298 |
| references/document-format.md | fc491d21de57d6ba7efe93f3658e67e3076dd9040405f6af07409b9b41d688a5 |
| references/ears.md | 6b9792a8054cb63519db7b8c666fe7cfb273ca2f23f932dddea6144614e9046a |
| references/elicitation-probes.md | 3360d7bd09d76fb81c6ed63a6355f013a847485646bb3e8ffe57a9b479c95e96 |
| references/quality-iso-25010.md | caa107db0b53859bed1acc73753dffe95451cb64ffc87f5ed138b45ae3128d3b |
| references/tracking-contract.md | 125ca954e1b6bdd9a76d916819773a8b0611c28b43e85471693fbb2edc6c3531 |
| references/tracking-gates.md | 355880829b04130f872f0827a1c6368d3343717900c3da29d11b2df6ed62ab13 |
| references/tracking.md | 0b11638a40321b66dcef950a9139845d8ece797f831d25757889fe3b240b966a |
| references/worked-example.md | ce66b4d3234f242e6cb9c48c671d367785249438349ace5824f03a1c97de623d |
| scripts/validate_spec.py | 9c52adef7d2575cb8aba268aee4928b0c7c8b09d827229e95f55b4745d24f1c5 |
| source-manifest.json | 863545cf225b6c1c080f57fbae1e6ac1a5ee204b2051d0221d820259466adeb0 |
| tests/fixtures/bad.md | bcb76ddbcaa35e4af542a47f09bec053ee77e875842df92e5f98f878693df581 |
| tests/fixtures/good.md | a19a1361cba4b2f27bff1385e0c5ed37cc9b1105ae0874816eb6c71f3ad96353 |
| tests/run.sh | 41e258f17ec56bd94a173af7f4aa2a7c81b2aa488abe0cff64ff774551a14cb2 |

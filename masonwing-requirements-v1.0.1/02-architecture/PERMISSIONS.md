# Permission/action matrix — Proposed A-SEC

Roles là presets,không authority riêng. Owner không bypass source rights,budget,approval hoặc platform forbid. Agent/service chỉ explicit delegation,không nhận mọi role capabilities. IdP admin không tự tenant Owner. Chi tiết action contract:

| Action | Feature | Candidate human roles | Step-up | Phase |
|---|---|---|---|---|
| product.compose | F-001 | OWNER,EDITOR | False | C1 |
| plugin.install | F-002 | OWNER | True | C1 |
| plugin.enable | F-003 | OWNER | True | C1 |
| plugin.disable | F-003 | OWNER | True | C1 |
| plugin.upgrade | F-003 | OWNER | True | C1 |
| plugin.revoke | F-003 | SECURITY_REVIEWER | True | C1 |
| plugin.uninstall | F-003 | OWNER | True | C1 |
| plugin.invoke | F-004 | OWNER,EDITOR | False | C1 |
| identity.configure | F-005 | PLATFORM_OPERATOR | True | C1 |
| membership.invite | F-006 | OWNER,EDITOR | False | C1 |
| membership.accept | F-006 | OWNER,EDITOR | False | C1 |
| membership.change | F-006 | OWNER,EDITOR | False | C1 |
| membership.revoke | F-006 | OWNER | True | C1 |
| policy.evaluate | F-007 | OWNER,EDITOR | False | C1 |
| policy.propose | F-007 | SECURITY_REVIEWER | True | C1 |
| grant.create | F-008 | OWNER,EDITOR | False | C1 |
| grant.revoke | F-008 | OWNER | True | C1 |
| artifact.begin | F-010 | OWNER,EDITOR | False | C1 |
| artifact.finalize | F-010 | OWNER,EDITOR | False | C1 |
| deletion.request | F-010 | OWNER,EDITOR | False | C1 |
| run.start | F-011 | OWNER,EDITOR | False | C1 |
| run.cancel | F-011 | OWNER,EDITOR | False | C1 |
| approval.decide | F-012 | OWNER,REVIEWER | False | C1 |
| effect.propose | F-013 | OWNER,EDITOR | False | C1 |
| effect.dispatch | F-013 | OWNER,PUBLISHER | False | C1 |
| effect.reconcile | F-013 | OWNER,EDITOR | False | C1 |
| effect.compensate | F-013 | OWNER,PUBLISHER | False | C1 |
| budget.configure | F-014 | OWNER | True | C1 |
| provider.configure | F-015 | OWNER | True | C1 |
| provider.compact | F-015 | OWNER,EDITOR | False | C1 |
| connection.authorize | F-016 | OWNER,EDITOR | False | C1 |
| connection.revoke | F-016 | OWNER | True | C1 |
| schedule.create | F-017 | OWNER,EDITOR | False | C1 |
| deadletter.replay | F-017 | OWNER,EDITOR | False | C1 |
| ui.register | F-018 | OWNER,EDITOR | False | C1 |
| export.create | F-019 | OWNER,EDITOR | False | C1 |
| notification.read | F-019 | OWNER,EDITOR | False | C1 |
| support.request | F-020 | OWNER | True | C1 |
| kill-switch.set | F-013 | OWNER,EDITOR | False | C1 |
| release.qualify | F-021 | PLATFORM_OPERATOR | True | C1 |
| catalog.submit | F-022 | OWNER,EDITOR | False | C2 |
| catalog.review | F-022 | SECURITY_REVIEWER | True | C2 |
| conformance.run | F-023 | OWNER,EDITOR | False | C2 |

Mọi public query cần read action cùng tenant/resource. Reviewer và Publisher tách khi four-eyes policy active. Các candidate roles không phải stakeholder-approved production policy;production activation đòi Security/PO approval.

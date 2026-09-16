# Masonwing development workspace

The two `*-requirements-v1.0.1` directories are immutable input baselines. Read the relevant SRS, architecture, contracts, work package and test cases before editing a slice. Keep their original bytes, IDs and acceptance thresholds. Implementing this scaffold does not approve production policy or provider activation.

Backend uses a Rust Cargo workspace. Browser packages use a pnpm workspace, React, TypeScript, shared UI and a Rust BFF. Kernel is domain neutral; Gleanbird and the document-review/checksum fixtures consume the SDK. Domain plugins cannot import private platform implementations, access another plugin's SQL, obtain raw secrets or transmit external mutations directly.

Put generated contracts under `contracts/` and clearly mark generated files. Regenerate and check drift instead of editing generated outputs. Preserve namespaced REQ/AC/TC links. Keep domain value objects, application ports and infrastructure adapters separate.

Tests assert observable results and denied side effects. Separate scaffold/contract/unit checks from the future product acceptance suite. An unimplemented case must remain RED or NOT_IMPLEMENTED; never turn skips, mock receipts or specification validation into product acceptance. Preserve Given/When/Then and negative oracles from the baseline.

The local Compose project is `masonwing-dev`. Publish only to loopback in 39850–39859. Do not stop, modify or claim ownership of another project's containers, ports or volumes. Local providers are synthetic; external mutation and live budget stay disabled. No real secrets in tracked files, browser bundles or evidence.

Use focused tests for changes and retain actual command results. Do not commit, push, publish, change production services or delete volumes without authorization. Preserve unrelated edits and coordinate file ownership between workers.

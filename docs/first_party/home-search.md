# Home + Search Workstream

This note records the Home (`M-APP-04`) and Search (`M-APP-07`) implementation
in `user/nagi-home-search`. The implementation is isolated from M17 and the
other first-party app worktrees.

## Boundaries

- `HomeController` projects `WorkspaceId` and `ObjectId` references from the
  shared `nagi-model`; `HomeDataSource` is the integration boundary for a Nagi
  Workspace service. The preview source is explicitly in-memory fixture data.
- `AppRegistry` is data-driven. `PackageRegistryAdapter` reads installed
  manifests from the existing M16 `PackageService`; installed packages remain
  unlaunchable until a user-space app launcher exists. `CompositeAppRegistry`
  merges registry sources and rejects duplicate app IDs.
- `SearchProvider` isolates apps, files, notes, activity, actions, and
  workspaces. `SearchCoordinator` applies provider selection, cancellation,
  deadlines, permission filters, deterministic ranking, identity deduplication,
  and result limits. File, note, activity, action, and workspace preview
  providers use fixtures and do not read host or guest user data.
- Search results keep presentation fields separate from typed app, object,
  workspace, and action identities. Opening a preview displays the typed
  target; it does not dispatch an OS action.
- The preview language selector changes only the preview locale. English
  (`en-US`) and Japanese (`ja-JP`) strings come from one catalog; missing
  Japanese entries fall back to English.

## Host preview

Run from the repository root:

```sh
cargo run --manifest-path user/nagi-home-search/Cargo.toml --bin home-search-preview
```

The server binds to `127.0.0.1:4173`. It embeds its HTML, CSS, and JavaScript,
uses a restrictive same-origin content policy, and serves only fixture-backed
Home and Search API responses. The preview includes first-party app statuses,
workspace references, files, notes, activity, actions, and workspaces. It
supports English/Japanese, keyboard search, result categories, cancellation,
and typed action inspection.

## Verification and remaining runtime integration

The focused Rust test suite covers the app registry and M16 package adapter,
workspace permission filtering, typed actions, localization, empty and
zero-result queries, exact/prefix/substring ranking, Unicode and Japanese,
multiple providers, provider failures/timeouts, explicit cancellation, stale
request suppression, capability filtering, duplicate identities, deterministic
ties, provider selection, and a 25,000-candidate result-limit fixture.

Nagi-target integration has not been run. The current runtime does not expose a
general Home Workspace data service, app process launcher, Search provider
services, or shared action dispatcher. Connect those services through the
interfaces above when they are available. This does not block the host core or
preview; fixture-backed results are never presented as Nagi production data.

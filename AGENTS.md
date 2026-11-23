# Famulen

一人の主人に仕える、執事や秘書のような AI の補佐役。

## How it is built

- **Test first.** Red → green → refactor. Write the failing test before the
  code that makes it pass.
- **Each layer holds one thing.** Put a fact in the one layer that owns it,
  and nowhere else.

  | Layer | Holds |
  | --- | --- |
  | `docs/concept.md` | what this is and for whom |
  | `docs/adr/` | one decision and why it was made |
  | `docs/spec/` | the behaviour users and other components can rely on |
  | Tests | the details that pin the behaviour down: rules, boundaries, tables |
  | Code | how it is done |
  | Commits | why a change was made |
  | Comments | why it is not done another way |

  Before writing, fixing or reviewing a record, read `docs/adr/README.md`;
  before writing, fixing or reviewing a spec, read `docs/spec/README.md`.
  Follow their writing order.
- **Dependencies are current.** Before adding or bumping one, look up its
  latest release in the registry. Do not copy a version from memory or from
  another repository.

## Quality gate

Run `just ci` before reporting a change as done. CI runs the same recipe,
and `just e2e` for anything the console shows.

## Workflow

Each change is made in a jj workspace of its own cut from `main`, and lands
through a pull request. `main` is never pushed to directly. A pull request
holds one intent, with its spec and tests, and is merged once CI is green.

## Layout

| Path | What lives there |
| --- | --- |
| `crates/core/` | `famulen-core`: the library the binary is built on |
| `crates/notebook/` | `famulen-notebook`: the notebook, its specification in `SPEC.md` and the library that keeps it |
| `crates/notebook-cli/` | `famulen-notebook-cli`: the `famulen-notebook` command, for people and AI to keep a notebook by its rules |
| `crates/api/` | `famulen-api`: the API the console and the secretary share, from `proto/` |
| `crates/gateway/` | `famulen-gateway`: the gateway to outside services, usable without Famulen; its promises in `SPEC.md` |
| `crates/gateway-cli/` | `famulen-gateway-cli`: the `famulen-gateway` command, for the owner to use a gateway directory without Famulen |
| `crates/cli/` | `famulen`: the binary |
| `proto/` | the API's definition in protobuf |
| `frontend/` | the console: a SolidStart app, its Playwright tests in `e2e/` |
| `Dockerfile`, `compose.yaml` | the containers the owner runs |
| `contrib/` | files the owner copies when setting Famulen up: the Slack app manifest, an example config, a systemd unit |
| `docs/` | concept, ADRs, specs, and how to register the apps (`setup/`) |
| `brand/` | the logo: mark, lockups, app icon, PNG exports and how to use them |
| `scripts/` | scripts the release and `just ci` run |

## Language

- README, docs and commit messages: Japanese
- Code comments, log and error messages: English
- The name is written Famulen wherever people read it: prose, the
  console, Slack. Identifiers stay lowercase: the `famulen` command, crate and
  directory names, `FAMULEN_*` variables, tool and action names.

## Commits

Scoped Commits:

```
<scope>: <description>

<why this change is needed>
```

- The scope names where the change is, from the table below. Several scopes
  are joined with `, `; a change across the whole tree uses `treewide`.
- The description starts in lower case, has no trailing period, and stays
  within 72 characters.
- The body explains why. The code shows how; the tests show what.
- A release commit is `release: vX.Y.Z`.
- A breaking change says so in the body and carries a `BREAKING CHANGE:`
  trailer.

| Scope | Covers |
| --- | --- |
| `core` | `crates/core/` |
| `notebook` | `crates/notebook/` |
| `notebook-cli` | `crates/notebook-cli/` |
| `cli` | `crates/cli/` |
| `api` | `crates/api/` |
| `gateway` | `crates/gateway/` |
| `gateway-cli` | `crates/gateway-cli/` |
| `proto` | `proto/` |
| `console` | `frontend/` |
| `deploy` | `Dockerfile`, `compose.yaml`, `.env.sample`, `contrib/` |
| `concept` | `docs/concept.md` |
| `adr` | `docs/adr/` |
| `spec` | `docs/spec/` |
| `docs` | `README.md`, `AGENTS.md`, `docs/setup/` |
| `ci` | `.github/` |
| `nix` | `flake.nix`, `flake.lock` |
| `build` | `justfile`, `Cargo.toml`, `rust-toolchain.toml`, `scripts/` |
| `deps` | dependency updates |
| `brand` | `brand/` |

Add a row when a new area appears.

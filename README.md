# Orchest

Orchest manages local development runtimes inside a configurable root. The current implementation is the first CLI milestone: PHP package installation, multiple installed versions, project-specific PHP selection, managed execution, JSON output, and diagnostics.

## Build

```sh
cargo build --workspace
cargo test --workspace
cargo run -p orchest-cli -- init
```

Use `--root PATH` or `ORCHEST_ROOT` to override the default root (`~/.local/share/orchest` on Linux; `%LOCALAPPDATA%\Orchest` on Windows). `init` creates configuration, state, logs, cache, and installation directories. It copies the built-in PHP manifest to `config/packages/`, where it can be edited without rebuilding.

## CLI

```sh
orchest init
orchest package list
orchest php install 8.4.15
orchest php install 8.5.10
orchest php default 8.5.10
orchest project add ./my-app --name my-app
orchest project php my-app 8.4.15
orchest exec php -v
orchest project exec my-app php -v
orchest port list
orchest port check 3306
orchest doctor --json
```

`orchest package install php@8.4.15` is equivalent to `orchest php install 8.4.15`. `--json`, `--quiet`, `--verbose`, and `--root` are global options. `orchest exec` changes `PATH` only for the child process and does not use a system PHP fallback.

For an offline or local release test, use `orchest php install 8.4.15 --archive /path/to/php-8.4.15-linux-x86_64.tar.gz`. The same archive validation and staged installation are used. Put global flags before `exec` when using `--json`, for example `orchest --json exec php -v`.

## Structure

```text
orchest-cli ─┐
             ├→ orchest-core → orchest-packages
orchest-api ─┘                → orchest-process
                             → orchest-platform
```

The reusable core owns project, config, and installation state. The package crate reads TOML manifests and installs from HTTPS archives via staging. The process crate runs managed binaries and provides a supervisor with PID identity checks and persistent logs; service commands have not been connected to it yet. The platform crate handles operating-system paths and atomic writes. The HTTP API calls the same core; a future Tauri adapter can do the same.

## Local API

Run `ORCHEST_ROOT=/path/to/root cargo run -p orchest-api` after `orchest init`. It listens only on `127.0.0.1`, port `8765` by default (`ORCHEST_API_PORT` overrides the port). The bearer token is created at `runtime/state/api-token` under the Orchest root. Supply it in `Authorization: Bearer <token>` for every request. Current `/api/v1` endpoints cover status, package catalog and installation, projects, PHP assignment, ports, and doctor. Service endpoints will be added when service supervision exists. Port checks test whether a loopback TCP bind succeeds; they do not identify another process using the port yet.

## PHP release artifacts

Windows uses the official PHP ZIP files listed in `manifests/php.toml`. Linux uses Orchest-built release archives because PHP does not publish official precompiled Linux binaries. The workflow in `.github/workflows/release-php-linux.yml` builds PHP CLI and FPM from PHP source in an Ubuntu 22.04 container, bundles non-glibc libraries, and tests the archive on Ubuntu 22.04, Ubuntu 24.04, and Debian 12 before publishing it to GitHub Releases.

Once this repository is on GitHub with Actions enabled, run **Release managed PHP for Linux** for version `8.4.15`, revision `1`, and then `8.5.10`, revision `1`. Set `orchest config set sources.github_repository owner/repo` in each Orchest installation. This selects the repository for the Linux release URLs; it does not change Windows downloads. New PHP versions require a manifest entry and a workflow run. Rebuilds use a new revision and a matching manifest URL. `config/packages/php.toml` in an existing Orchest root is user-controlled and is not overwritten by `init`.

Each artifact requires `url`, `archive` (`zip` or `tar.gz`), and `executable`; `strip_components` is optional. Managed PHP on Windows may need the Microsoft Visual C++ runtime supplied by the OS or installed separately. The release workflow has been tested locally for PHP 8.4.15 and 8.5.10, but the assets have not been published because this directory has no Git remote and the local GitHub CLI token is invalid.

## Current limits

This is an early CLI and API slice. The Linux build and distribution workflow exists but has not been run in GitHub Actions or published. The process supervisor is not connected to service commands yet; Nginx/Apache/database integrations, certificates, and desktop UI remain to be implemented. The API currently runs synchronous core operations directly for its short handlers; long downloads use a blocking worker.

# Orchest

Orchest manages local development runtimes inside a configurable root. The current CLI and API cover PHP, Nginx, Apache, local HTTPS, Mailpit, Meilisearch, package installation, projects, and diagnostics.

## Build

Run these commands on **each target operating system**. Cargo builds for the current host unless a Rust target and its linker are configured explicitly.

```sh
cargo build --workspace
cargo test --workspace
cargo build --release -p orchest-cli -p orchest-api
```

The release binaries are `target/release/orchest` and `target/release/orchest-api` on Linux, or `target\release\orchest.exe` and `target\release\orchest-api.exe` on Windows. The [Rust CI workflow](.github/workflows/rust-ci.yml) compiles and tests the workspace on Ubuntu 24.04 and Windows Server 2022 when started manually with **Run workflow** in GitHub Actions. It does not run on pushes or pull requests, to control billed runner minutes. The workflow uploads the CLI and API binaries as downloadable Actions artifacts (`orchest-linux-x86_64` and `orchest-windows-x86_64`) for testing. Windows VM installation and managed PHP validation were confirmed by the user. Building the Rust applications does not build PHP: managed PHP artifacts are obtained separately from the package manifest.

Available [Orchest prereleases](https://github.com/danidoble/orchest/releases) contain both executables and an installation script in each Linux (`.tar.gz`) and Windows (`.zip`) package, plus `SHA256SUMS.txt`. New prereleases are built only by the manually triggered [release workflow](.github/workflows/release-orchest.yml); the ordinary Rust CI is also manual to control runner minutes. Run `./publish.sh` from a clean checkout to prepare the next alpha version, verify it locally, push it and start that workflow with separate confirmations. Use `./publish.sh --resume` after fixing a failed local check, or `./publish.sh --retry` if a prepared version needs another workflow attempt.

## Install Orchest commands

The Linux archive includes `install-linux.sh` alongside `orchest` and `orchest-api`. Run `bash install-linux.sh` from the extracted archive. It copies both commands to `~/.local/bin` and adds that directory to shell startup files. Open a new terminal and run `orchest init`. It does not use `sudo` or modify system PHP installations.

This Linux installation flow was verified inside a disposable Ubuntu 26.04 container; `orchest` was found on `PATH` and `init` created `www/` there.

The Windows ZIP includes `install-windows.ps1` alongside both `.exe` files. From an **Administrator PowerShell** in the extracted directory, run `powershell -ExecutionPolicy Bypass -File .\install-windows.ps1`. The script downloads Microsoft's x64 Visual C++ Redistributable, verifies its Authenticode signature, installs it, copies the Orchest executables to `C:\Program Files\Orchest`, and adds that directory to the machine `PATH`. Open a new normal-user terminal and run `orchest init`. The installer does not initialize the elevated administrator's data directory. Windows VM installation and Redistributable validation were confirmed by the user.

Use `--root PATH` or `ORCHEST_ROOT` to override the default root (`~/.local/share/orchest` on Linux; `%LOCALAPPDATA%\Orchest` on Windows). `init` creates configuration, state, logs, cache, installation directories, and a default `www/` directory. It copies bundled package manifests to `config/packages/`, where custom sources and versions can be added. Running `init` again merges newly bundled PHP versions into an existing PHP manifest while keeping custom version URLs; it updates the old built-in `{github_repository}` URLs to the public release URLs.

## CLI

```sh
orchest init
orchest package list
orchest php install 8.4.15
orchest php install 8.5.10
orchest php default 8.5.10
orchest project add --name my-new-app
orchest project add ./my-app --name my-app
orchest project php my-app 8.4.15
orchest exec php -v
orchest project exec my-app php -v
orchest package install mailpit@1.31.1
orchest service start mailpit
orchest service status mailpit
orchest service stop mailpit
orchest package install meilisearch@1.51.0
orchest service start meilisearch
orchest service status meilisearch
orchest service stop meilisearch
orchest package install nginx@1.30.5
orchest package install apache@2.4.68
orchest php install 8.4.15
orchest project add --name demo
orchest project php demo 8.4.15
orchest project ssl demo on
orchest service config nginx
orchest service start nginx
orchest service reload nginx
orchest service status nginx
orchest service status php@8.4.15
orchest php extensions 8.4.15
orchest project add --name legacy
orchest project web-server legacy apache
orchest project ssl legacy on
orchest service config apache
orchest ssl renew
orchest service stop nginx
orchest port list
orchest port check 3306
orchest doctor --json
```

`orchest package install php@8.4.15` is equivalent to `orchest php install 8.4.15`. `project add --name NAME` creates and registers `<root>/www/NAME`; supplying a path continues to register any existing directory. `--json`, `--quiet`, `--verbose`, and `--root` are global options. `orchest exec` changes `PATH` only for the child process and does not use a system PHP fallback.

`orchest doctor` checks that the root is writable, reads configuration, runs SQLite `quick_check`, validates installed executables and project paths, reports unavailable configured ports, and flags stale service instance records. It does not remove those records or stop processes. A SQLite port registry records the service and instance that claimed each port. `port list` and `port check` expose `owner` and `state` (`available`, `managed`, `reserved`, `stale_claim`, or `unavailable`). External port owners are reported as unavailable without claiming a PID.

Nginx listens on loopback ports 80 and 443 by default and serves registered projects at `<project>.test`. Add a hosts-file entry for each domain or send a matching `Host` header. HTTPS uses a local Orchest CA under `<root>/certificates/ca`; certificate trust in the operating system is optional and remains a manual choice. Certificates are renewed before expiry while Nginx is running, and `orchest ssl renew` checks them immediately. `orchest service config nginx` previews the configuration; the API exposes `GET /api/v1/services/nginx/config`. Starting Nginx validates with `nginx -t`, starts FastCGI backends per PHP version, and reserves both entry ports. Linux uses managed `php-fpm`; Windows uses `php-cgi.exe`. Each FastCGI backend binds only to loopback on a persistent port in `19000..19999`. PHP changes through CLI/API validate and reload Nginx automatically. Projects without PHP serve static files. Nginx with FastCGI is the default for every PHP project. Only a project explicitly configured with `orchest project web-server NAME apache` starts Apache and routes that site's requests through Nginx to Apache on `127.0.0.1:8080`, preserving Host and forwarding the client IP. On Linux, ports 80/443 require suitable privileges; configurable higher ports work without them.

The previous Nginx Linux release (`r1`) lacks SSL support. The manifest points to an SSL-enabled `r2` archive. The Apache Linux manifest points to its first `r1` archive, and PHP Linux manifests point to extension-enabled `r2` archives. These Release assets must be published before online installation; meanwhile, use `--archive` with locally built archives for isolated testing. The Windows Nginx ZIP is official; the Apache ZIP is from [Apache Lounge](https://www.apachelounge.com/download/), which the [Apache HTTP Server documentation](https://httpd.apache.org/docs/2.4/platform/windows.html) lists as a Windows binary provider. See [PHP extensions](docs/php-extensions.md) for module coverage and Windows Imagick packages.

For an offline or local release test, use `orchest php install 8.4.15 --archive /path/to/php-8.4.15-linux-x86_64.tar.gz`. The same archive validation and staged installation are used. Put global flags before `exec` when using `--json`, for example `orchest --json exec php -v`.

## Structure

```text
orchest-cli ─┐
             ├→ orchest-core → orchest-packages
orchest-api ─┘                → orchest-process
                             → orchest-platform
```

The reusable core owns project, config, installation, and port claim state. The package crate reads TOML manifests and installs HTTPS archives or direct executable assets via staging. The process crate runs managed binaries and provides a supervisor with PID identity checks and persistent logs. The supervisor supports multiple named instances per service, isolated state and logs, instance listing, and explicit cleanup of stale records. Existing Mailpit records remain the `default` instance. Mailpit and Meilisearch are managed through the same core. The platform crate handles operating-system paths and atomic writes. The HTTP API calls the same core; a future Tauri adapter can do the same.

## Local API

Run `ORCHEST_ROOT=/path/to/root cargo run -p orchest-api` after `orchest init`. It listens only on `127.0.0.1`, port `8765` by default (`ORCHEST_API_PORT` overrides the port). The bearer token is created at `runtime/state/api-token` under the Orchest root. Supply it in `Authorization: Bearer <token>` for every request. Current `/api/v1` endpoints cover config read/update, package catalog and installation, projects and their PHP/web server/SSL settings, loaded PHP extensions, ports, doctor, SSL renewal, and service status/start/stop/configuration. Mailpit listens on loopback ports `8025` (HTTP) and `1025` (SMTP) by default. Meilisearch listens on loopback port `7700` by default. Port checks distinguish a registered managed instance from an unavailable external port, but do not identify the external process PID.

## PHP release artifacts

Windows uses the official PHP ZIP files listed in `manifests/php.toml`. Linux uses Orchest-built release archives because PHP does not publish official precompiled Linux binaries. The workflow in `.github/workflows/release-php-linux.yml` builds PHP CLI and FPM from PHP source in an Ubuntu 22.04 container, bundles non-glibc libraries, and tests the archive on Ubuntu 22.04, Ubuntu 24.04, and Debian 12 before publishing it to GitHub Releases.

The repository is public. The bundled [PHP manifest](manifests/php.toml) lists Windows ZIPs and direct Linux release URLs for PHP 8.2.33, 8.3.33, 8.4.15, and 8.5.10. No manual GitHub repository setting is needed. Custom package manifests and `--archive` remain available for versions or sources outside this catalog. Rebuilds use a new packaging revision and a matching manifest URL.

Each artifact requires `url`, `archive` (`zip`, `tar.gz`, or `binary`), and `executable`; `strip_components` is optional for archives. Windows PHP ZIPs require Microsoft's Visual C++ runtime, which the new Windows installer handles. On Linux, `orchest php install 8.2.33` downloads directly from the public release URL; `--archive PATH` remains for external or offline archives. All four versions use isolated directories under the Orchest root.

Direct HTTPS installation and CLI execution of all four Linux versions passed in an isolated Ubuntu 26.04 container. This did not install any PHP version into the host system.

## Current limits

This is an early CLI and API slice. The Nginx → Apache/PHP FastCGI path, HTTPS, PHP switch/reload, and automatic certificate renewal passed an isolated Debian 12 smoke test. Windows Apache/HTTPS and the newly enabled PHP extensions still need VM testing. Node/Corepack, databases, schedulers, queues, and desktop UI remain pending. The API runs downloads and web reconfiguration in blocking workers; short handlers still call the core directly.

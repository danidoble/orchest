# Orchest — CLI-First Local Development Environment Orchestrator

## Seguimiento de implementación — 2026-09-16

Esta lista registra el estado real del proyecto. `[x]` significa implementado y verificado en el entorno indicado; `[ ]` significa pendiente. Los requisitos detallados siguen en las secciones originales de este documento. La sección 32 describe el objetivo inicial, no su estado actual.

### Hito 1: CLI y PHP administrado (secciones 1–10, 17–18, 23–28)

- [x] Workspace Cargo con `core`, `cli`, `api`, `platform`, `packages` y `process`; configuración TOML y estado de paquetes/proyectos en SQLite.
- [x] Catálogo PHP por manifest, descarga HTTPS, extracción ZIP/TAR.GZ con rechazo de rutas inseguras, staging, validación del ejecutable e instalación atómica.
- [x] Registro de versiones instaladas, alias `orchest php`, desinstalación protegida cuando una versión está asignada, defaults, proyectos y resolución PHP por proyecto.
- [x] `orchest exec` y `project exec` sin modificar el PATH global; salida JSON, `init`, `status`, `config`, `doctor` y comprobación básica de puertos.
- [x] Builds Linux locales de PHP 8.4.15 y 8.5.10, con CLI y FPM; ejecución probada en Debian 12 y Ubuntu 24.04. Secuencia del hito probada en Linux con `--archive` y ambas versiones reales.
- [x] ZIP oficiales de PHP para Windows declarados en el manifest.
- [x] Publicar los artefactos Linux de PHP 8.4.15 y 8.5.10 en GitHub Releases; verificar digest SHA-256, instalación desde los assets descargados y ejecución de CLI y FPM en Linux.
- [ ] Probar `orchest php install <versión>` desde la URL HTTPS del manifest, sin `--archive`, tras resolver la autenticación del repositorio privado.
- [ ] Ejecutar la secuencia completa del hito en Windows con los ZIP oficiales y verificar `php.ini`, rutas con espacios y dependencias de Visual C++.
- [ ] Ampliar `doctor` con escritura de raíz, integridad SQLite, puertos ocupados por otros procesos y estados PID obsoletos.

### Distribución y plataformas (secciones 3–6, 26–27, 30)

- [x] Workflow local de build Linux desde fuentes oficiales de PHP, empaquetado de bibliotecas y pruebas en Ubuntu 22.04/24.04 y Debian 12; workflow de GitHub Releases preparado.
- [x] Remoto privado confirmado: `danidoble/orchest`. La autenticación local de `gh` funciona fuera del sandbox.
- [x] CI manual (`workflow_dispatch`) para compilar, probar y ejecutar Clippy en runners Linux y Windows x64; sin ejecución automática en cada push o pull request para controlar el consumo de minutos.
- [x] Primera ejecución verde de CI en Linux y Windows: run `35134779304` del 2026-09-16, con compilación release de CLI y API.
- [x] CI run `35140228611`: binarios CLI y API adjuntos como artifacts descargables para Linux y Windows; archivos descargados y formatos verificados, CLI Linux ejecutada.
- [ ] Resolver descargas autenticadas de assets de GitHub Releases mientras el repositorio sea privado, o mantener la instalación local con `--archive` hasta hacerlo público. No almacenar tokens en TOML ni logs.
- [ ] Publicar y probar binarios `orchest` y `orchest-api` para Windows y Linux; definir instaladores y actualización del propio Orchest.
- [ ] Versionar los artefactos Linux por revisión y actualizar el manifest al reconstruir una versión; automatizar la comprobación de URLs y compatibilidad antes de publicar.

### Servicios, proyectos web y datos (secciones 11–16, 19, 24–25)

- [x] Base de supervisor de procesos con PID, identidad del ejecutable, logs y parada; prueba de inicio/parada en Linux.
- [ ] Conectar el supervisor a comandos y API de servicios; persistir instancias, detectar caídas y probar procesos/árboles en Windows y Linux.
- [ ] Añadir manifests, instalación autocontenida y configuraciones para Nginx, Apache, MySQL, MariaDB, MongoDB, Redis y Node; preservar varias versiones e instancias.
- [ ] Implementar registro de puertos con propietario real, reservas y diagnóstico de conflictos sin finalizar procesos ajenos.
- [ ] Implementar proxy de entrada 80/443, selección Nginx/Apache por proyecto y configuraciones generadas y validadas.
- [ ] Implementar PHP-FPM por versión en Linux y FastCGI con `php-cgi.exe` en Windows; enrutar cada proyecto a su versión PHP.
- [ ] Implementar datos persistentes por instancia, inicio/parada de proyectos, dominios `.test`, hosts y certificados locales con confianza opcional.
- [ ] Añadir eventos del núcleo para progreso de instalaciones y cambios de procesos/proyectos.

### API, escritorio y aceptación (secciones 20–22, 29–31)

- [x] API local Axum con rutas `/api/v1` para operaciones ya disponibles, enlace loopback y bearer token; probado `401` sin token y estado con token.
- [ ] Completar rutas y DTO de servicios, proyectos web, datos y eventos; probar errores y seguridad de la API.
- [ ] Crear adaptador Tauri y frontend React/TypeScript/shadcn después de validar los flujos CLI y API, sin lógica de orquestación en la UI.
- [ ] Probar el recorrido completo en Windows y Linux: instalación HTTPS, dos versiones PHP, proyecto web, servicios, base de datos, reinicio, recuperación y desinstalación segura.

---

Build a production-quality cross-platform local development environment orchestrator named **Orchest**.

Orchest is conceptually inspired by tools such as Laravel Herd, Laragon and XAMPP, but its architecture must be designed from scratch around these principles:

- Rust-first.
- CLI-first.
- Windows and Linux first-class support.
- Self-contained managed runtimes.
- Multiple versions of the same runtime/service installed simultaneously.
- Per-project runtime and service selection.
- No dependency on system-global PHP, Nginx, Apache, MySQL, MariaDB, MongoDB, Redis, Node.js, etc.
- A reusable Rust core that can later power:
  - the CLI,
  - a local HTTP API,
  - Tauri commands,
  - a React + shadcn desktop frontend.
- The future frontend must contain no important orchestration logic.
- The Rust backend must remain usable without Tauri.

The first milestone is **NOT the GUI**.

The first milestone is a complete, usable CLI and backend architecture.

---

# 1. Main Goal

Orchest must manage isolated local development toolchains from its own directory.

Example Windows installation root:

```text
C:\Orchest\
```

Example Linux installation root:

```text
~/.local/share/orchest/
```

The root must be configurable.

Example layout:

```text
Orchest/
├── bin/
│   ├── php/
│   │   ├── 8.1.32/
│   │   ├── 8.2.28/
│   │   ├── 8.3.20/
│   │   ├── 8.4.15/
│   │   └── 8.5.10/
│   │
│   ├── nginx/
│   │   ├── 1.26.3/
│   │   └── 1.28.0/
│   │
│   ├── apache/
│   ├── mysql/
│   ├── mariadb/
│   ├── mongodb/
│   ├── redis/
│   ├── node/
│   ├── composer/
│   └── tools/
│
├── data/
│   ├── mysql/
│   ├── mariadb/
│   ├── mongodb/
│   ├── redis/
│   └── services/
│
├── config/
│   ├── orchest.toml
│   ├── services/
│   ├── projects/
│   └── packages/
│
├── logs/
│   ├── orchest/
│   ├── nginx/
│   ├── apache/
│   ├── mysql/
│   └── projects/
│
├── runtime/
│   ├── pid/
│   ├── sockets/
│   ├── generated/
│   └── state/
│
├── cache/
│   └── downloads/
│
├── certificates/
│   ├── ca/
│   └── sites/
│
└── backups/
```

All software managed by Orchest should remain inside this directory whenever technically possible.

Do not install managed runtimes globally.

Do not modify the user's global PATH unless explicitly requested.

---

# 2. Architecture

Use a Cargo workspace.

Recommended initial workspace:

```text
orchest/
├── Cargo.toml
├── crates/
│   ├── orchest-core/
│   ├── orchest-cli/
│   ├── orchest-api/
│   ├── orchest-platform/
│   ├── orchest-packages/
│   └── orchest-process/
│
├── manifests/
├── examples/
├── tests/
└── docs/
```

Responsibilities:

## orchest-core

Contains business logic and public domain APIs.

It must know about:

- projects,
- runtimes,
- services,
- versions,
- configuration,
- installation,
- environments,
- ports,
- domains,
- certificates,
- runtime resolution.

It must NOT depend on:

- Clap,
- Tauri,
- React,
- HTTP-specific concepts.

Example public APIs:

```rust
install_package(...)
remove_package(...)
list_installed_versions(...)
resolve_runtime(...)
create_project(...)
update_project(...)
start_service(...)
stop_service(...)
restart_service(...)
service_status(...)
assign_runtime(...)
generate_project_environment(...)
```

---

## orchest-cli

CLI interface using `clap`.

It should translate CLI commands into calls to `orchest-core`.

No important business logic should live in this crate.

---

## orchest-api

Local HTTP API.

Use Axum.

This crate should expose the same capabilities as the CLI through JSON endpoints.

It should be implemented after the core CLI functionality exists, but prepare the architecture for it immediately.

Example:

```text
GET    /api/v1/status
GET    /api/v1/packages
GET    /api/v1/packages/php
POST   /api/v1/packages/php/install
DELETE /api/v1/packages/php/{version}

GET    /api/v1/projects
POST   /api/v1/projects
GET    /api/v1/projects/{id}
PATCH  /api/v1/projects/{id}

POST   /api/v1/projects/{id}/start
POST   /api/v1/projects/{id}/stop

GET    /api/v1/services
POST   /api/v1/services/{id}/start
POST   /api/v1/services/{id}/stop
POST   /api/v1/services/{id}/restart
```

Do not duplicate logic between CLI and API.

Both must invoke `orchest-core`.

---

## orchest-platform

Platform abstractions.

Support:

```rust
enum Platform {
    Windows,
    Linux,
}
```

Handle platform-specific behavior such as:

- paths,
- process creation,
- process termination,
- filesystem permissions,
- symbolic links,
- Windows junctions where useful,
- executable extensions,
- archive extraction,
- localhost configuration,
- hosts file location,
- certificate trust,
- process trees.

Keep `#[cfg(target_os = "...")]` mostly isolated here instead of spreading it throughout the project.

---

## orchest-packages

Package catalog and installation engine.

Responsible for:

- available versions,
- platform downloads,
- archive type,
- executable paths,
- install layout,
- configuration templates,
- dependencies,
- runtime metadata.

---

## orchest-process

Process supervision.

Responsible for:

- spawning services,
- tracking PID,
- stopping services,
- restarting services,
- capturing stdout/stderr,
- log files,
- detecting crashed processes,
- process status,
- graceful shutdown,
- force termination fallback.

Do not make Orchest depend on systemd or Windows Services for normal local development processes.

Orchest should supervise its own processes.

---

# 3. Technology Choices

Prefer:

```text
clap
tokio
reqwest
serde
serde_json
toml
tracing
tracing-subscriber
thiserror
anyhow
uuid
chrono
directories
tempfile
zip
tar
flate2
axum
tower
tower-http
```

Choose additional libraries only when justified.

Prefer explicit, understandable code over excessive abstraction.

---

# 4. Package Manifest System

This is extremely important.

Do NOT hardcode every package/version inside Rust source code.

Create a manifest-driven package catalog.

For example:

```text
manifests/
├── php.toml
├── nginx.toml
├── apache.toml
├── mysql.toml
├── mariadb.toml
├── mongodb.toml
├── redis.toml
└── node.toml
```

Example concept:

```toml
[package]
id = "php"
name = "PHP"
type = "runtime"

[[versions]]
version = "8.5.10"

[versions.windows-x86_64]
url = "https://downloads.php.net/~windows/releases/archives/php-8.5.10-nts-Win32-vs17-x64.zip"
archive = "zip"
executable = "php.exe"

[versions.linux-x86_64]
url = "https://example.org/php/linux/php-8.5.10-x86_64.tar.gz"
archive = "tar.gz"
executable = "bin/php"
```

This example URL should demonstrate the desired package mechanism.

The package manager must not require developers to modify Rust code just to add another PHP version.

Adding a runtime version should normally mean updating package metadata.

---

# 5. Downloads and Integrity Policy

For the first implementation, **do not require checksum validation**.

Many upstream projects do not provide consistent machine-readable checksums across all versions and platforms, and maintaining our own checksum database would create unnecessary maintenance overhead.

Package manifests therefore do NOT require SHA256, SHA512, signatures, or checksum fields.

Installation flow:

```text
manifest URL
   ↓
HTTPS download
   ↓
temporary file
   ↓
archive extraction
   ↓
basic installation validation
   ↓
atomic move into Orchest/bin/<package>/<version>
```

Do not reject packages merely because no checksum exists.

However:

- use HTTPS URLs by default;
- never execute files directly from the download cache;
- extract into a temporary staging directory first;
- reject path traversal archive entries;
- validate that expected binaries exist after extraction;
- only then move the package to its final directory;
- make checksum/signature verification an optional future capability, not a mandatory requirement.

For example:

```text
C:\Orchest\cache\downloads\php-8.5.10.zip
```

should eventually result in:

```text
C:\Orchest\bin\php\8.5.10\
```

---

# 6. Package Installation State

Keep metadata describing installed packages.

Example concept:

```json
{
  "package": "php",
  "version": "8.5.10",
  "platform": "windows-x86_64",
  "installed_at": "2026-09-16T12:00:00Z",
  "source_url": "https://downloads.php.net/~windows/releases/archives/php-8.5.10-nts-Win32-vs17-x64.zip",
  "executable": "php.exe"
}
```

Do not determine installed versions solely by scanning directories.

Directories can be inspected for recovery, but maintain explicit state.

---

# 7. CLI Design

The CLI executable must be:

```text
orchest
```

The CLI should feel similar to tools such as Docker, Git, Cargo, Rustup or Symfony CLI.

Use predictable noun/verb commands.

Examples:

```bash
orchest status

orchest doctor

orchest config show

orchest package list

orchest package search php

orchest package install php@8.5.10

orchest package remove php@8.5.10

orchest package installed

orchest php list

orchest php install 8.5.10

orchest php default 8.4

orchest project list

orchest project add ./my-project

orchest project show my-project

orchest project php my-project 8.3

orchest project server my-project nginx

orchest project database my-project mysql

orchest project start my-project

orchest project stop my-project

orchest service list

orchest service start nginx

orchest service stop nginx

orchest service restart nginx

orchest logs nginx

orchest logs project my-project
```

Support machine-readable output:

```bash
orchest project list --json
```

The CLI should support:

```text
--json
--quiet
--verbose
--root
```

where appropriate.

---

# 8. Runtime Resolution

This feature is fundamental.

Multiple versions of PHP must coexist.

Example:

```text
C:\Orchest\bin\php\8.2.28\
C:\Orchest\bin\php\8.3.20\
C:\Orchest\bin\php\8.4.15\
C:\Orchest\bin\php\8.5.10\
```

Orchest must support:

```text
Global PHP: 8.4
```

while projects can use:

```text
shop       → PHP 8.3
legacy-crm → PHP 8.1
api        → PHP 8.4
sandbox    → PHP 8.5
```

Create a runtime resolver.

Conceptually:

```rust
pub struct RuntimeResolver;

impl RuntimeResolver {
    pub async fn resolve_php(
        &self,
        project: Option<&Project>,
    ) -> Result<RuntimeInstallation>;
}
```

Resolution order:

```text
project-specific version
        ↓
global Orchest version
        ↓
error
```

Never fall back to system PHP unless the user explicitly enables that behavior.

---

# 9. Command Execution

Eventually this must work:

```bash
orchest exec php -v
```

Using the global PHP version.

And:

```bash
orchest exec --project mi-tienda php artisan migrate
```

Using that project's configured PHP version.

Also:

```bash
orchest project exec mi-tienda php artisan migrate
```

Resolve binaries internally.

Do NOT require PATH mutation.

Construct the process environment dynamically.

Example:

```text
PATH =
C:\Orchest\bin\php\8.3.20;
C:\Orchest\bin\node\22.0.0;
<existing PATH>
```

only for the child process.

---

# 10. Projects

Define a persistent project model.

Example:

```rust
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub path: PathBuf,
    pub domain: Option<String>,
    pub php_version: Option<String>,
    pub web_server: Option<WebServer>,
    pub database: Option<DatabaseBinding>,
    pub node_version: Option<String>,
    pub ssl_enabled: bool,
}
```

Store project configuration in a durable format.

SQLite is acceptable for the internal application database.

Project-specific human-readable configuration files may also be generated.

Do not depend solely on `.env`.

---

# 11. Service Model

Create a generic service abstraction.

For example:

```rust
pub trait ManagedService {
    fn id(&self) -> &str;

    async fn start(&self) -> Result<()>;

    async fn stop(&self) -> Result<()>;

    async fn restart(&self) -> Result<()>;

    async fn status(&self) -> Result<ServiceStatus>;
}
```

Support initially:

```text
nginx
apache
mysql
mariadb
mongodb
redis
```

Later:

```text
mailpit
minio
meilisearch
postgresql
rabbitmq
```

Do not design a huge plugin system yet.

First create a stable service abstraction that can later support external providers.

---

# 12. Process Supervision

Every managed process should be owned logically by Orchest.

Keep runtime state such as:

```text
runtime/pid/nginx.pid
runtime/pid/mysql.pid
```

but do not trust PID files alone.

Validate that the process actually exists.

Store:

```rust
pub struct ProcessState {
    pub service_id: String,
    pub pid: u32,
    pub executable: PathBuf,
    pub started_at: DateTime<Utc>,
}
```

Capture logs to Orchest's log directory.

For example:

```text
logs/nginx/stdout.log
logs/nginx/stderr.log
```

---

# 13. Port Management

Create a port registry.

Orchest must detect:

- whether a port is free,
- which Orchest service owns a configured port,
- whether another process is already using it.

Example:

```bash
orchest port list
```

Output:

```text
80      nginx       running
443     nginx       running
3306    mysql       running
6379    redis       running
8025    mailpit     running
```

And:

```bash
orchest port check 3306
```

Do not silently kill unrelated processes occupying a port.

---

# 14. Web Servers

Support both Nginx and Apache.

Projects can independently choose their server.

Example:

```text
mi-tienda    nginx
api-rest     nginx
legacy-app   apache
```

Use Orchest-generated configuration.

Example:

```text
runtime/generated/nginx/sites/mi-tienda.conf
runtime/generated/apache/sites/legacy-app.conf
```

Never require users to manually edit the primary Nginx or Apache configuration for normal projects.

---

# 15. PHP-FPM

Design PHP web execution around independent PHP versions.

Nginx projects may point to different PHP-FPM instances.

For example:

```text
PHP 8.2 FPM → port 9082
PHP 8.3 FPM → port 9083
PHP 8.4 FPM → port 9084
PHP 8.5 FPM → port 9085
```

This is only an example strategy.

The implementation should abstract FPM endpoints so ports can be allocated dynamically later.

Nginx should route each project to its configured PHP runtime.

Example:

```text
legacy.test
    ↓
Nginx
    ↓
PHP 8.2 FPM
```

while:

```text
api.test
    ↓
Nginx
    ↓
PHP 8.5 FPM
```

This is one of Orchest's core differentiators.

---

# 16. Databases

Initially support:

```text
MySQL
MariaDB
MongoDB
Redis
```

Keep their data under Orchest.

Example:

```text
data/mysql/default/
data/mariadb/default/
data/mongodb/default/
data/redis/default/
```

Design the data model so multiple instances can be supported later:

```text
data/mysql/mysql-8.4-default/
data/mysql/mysql-9.0-testing/
```

Do not make single-instance assumptions deep inside the domain model.

---

# 17. Configuration

Main configuration:

```text
config/orchest.toml
```

Example:

```toml
[orchest]
root = "C:\\Orchest"

[defaults]
php = "8.4"
web_server = "nginx"
domain_suffix = "test"

[ports]
nginx_http = 80
nginx_https = 443
mysql = 3306
mariadb = 3307
mongodb = 27017
redis = 6379
```

Provide:

```bash
orchest config show
orchest config get defaults.php
orchest config set defaults.php 8.5
```

Validate configuration before writing it.

Use atomic writes.

---

# 18. Errors

Create typed errors.

Do not return stringly-typed errors everywhere.

For example:

```rust
pub enum OrchestError {
    PackageNotFound,
    VersionNotFound,
    DownloadFailed,
    ExtractionFailed,
    InvalidPackage,
    RuntimeNotInstalled,
    PortInUse,
    ProcessStartFailed,
    ProcessStopFailed,
    ProjectNotFound,
    InvalidConfiguration,
}
```

Map these errors cleanly to:

- CLI output,
- JSON output,
- future HTTP API errors.

---

# 19. Events

Prepare the core for future frontend realtime updates.

Create an event abstraction.

Examples:

```rust
pub enum OrchestEvent {
    PackageDownloadStarted,
    PackageDownloadProgress,
    PackageInstalled,
    PackageRemoved,

    ServiceStarting,
    ServiceStarted,
    ServiceStopping,
    ServiceStopped,
    ServiceCrashed,

    ProjectUpdated,
}
```

Do not tie this to Tauri events.

Use a generic event bus/channel internally.

Later:

```text
Core Event
    ↓
API WebSocket / SSE

or

Core Event
    ↓
Tauri event
```

---

# 20. Future Tauri Integration

Do not implement the GUI yet.

But architecture must support:

```text
React + TypeScript + shadcn
            ↓
         Tauri
            ↓
      Rust adapter
            ↓
       orchest-core
```

or:

```text
React
   ↓
localhost HTTP API
   ↓
orchest-api
   ↓
orchest-core
```

Ideally support both.

Tauri commands should be thin adapters such as:

```rust
#[tauri::command]
async fn list_projects(
    state: State<'_, AppState>,
) -> Result<Vec<ProjectDto>, ApiError> {
    state.orchest.projects().list().await
}
```

No Tauri command should contain service orchestration logic.

---

# 21. API Design

The API should use versioned routes:

```text
/api/v1/
```

Use DTOs separate from internal domain structs where useful.

Example response:

```json
{
  "id": "php",
  "installed_versions": [
    {
      "version": "8.4.15",
      "global": true
    },
    {
      "version": "8.5.10",
      "global": false
    }
  ]
}
```

Keep API naming consistent with CLI concepts.

For example:

```text
CLI:
orchest service start nginx

API:
POST /api/v1/services/nginx/start

Core:
service_manager.start("nginx")
```

All three represent the same operation.

---

# 22. Security Boundary

The API must bind to localhost only by default:

```text
127.0.0.1
```

Never expose Orchest's control API to the LAN by default.

Design authentication/token support for later, even if initial localhost-only development does not require complex authentication.

Avoid shell command interpolation.

Prefer:

```rust
Command::new(binary)
    .arg(...)
```

instead of:

```text
sh -c "..."
```

or:

```text
cmd /C "..."
```

whenever possible.

---

# 23. Installer Workflow

For:

```bash
orchest php install 8.5.10
```

Implement approximately:

```text
CLI
 ↓
PackageManager
 ↓
resolve manifest
 ↓
resolve OS + architecture
 ↓
determine URL
 ↓
download to cache
 ↓
extract to temporary staging directory
 ↓
validate expected binary
 ↓
prepare default configuration
 ↓
atomic install
 ↓
register installed package
 ↓
emit PackageInstalled event
```

Example final path:

```text
C:\Orchest\bin\php\8.5.10\
```

The installer must be idempotent where reasonable.

Installing an already-installed package should not corrupt it.

---

# 24. Uninstall Workflow

Example:

```bash
orchest php remove 8.5.10
```

Before removal:

- determine whether projects use the version;
- determine whether it is the global version;
- display dependencies;
- refuse unsafe removal unless `--force` is provided.

Example:

```text
Cannot remove PHP 8.5.10.

Used by:
  api
  sandbox

Use another PHP version first or pass --force.
```

---

# 25. Doctor Command

Implement:

```bash
orchest doctor
```

Check:

- root directory writable,
- config readable,
- package state readable,
- managed binaries present,
- expected executable paths valid,
- ports,
- zombie PID records,
- project paths,
- hosts permissions where relevant,
- certificates later.

Example:

```text
Orchest Doctor

[OK] Orchest root
[OK] Configuration
[OK] PHP 8.4.15
[OK] PHP 8.5.10
[OK] Nginx
[WARN] Port 3306 already used by another process
[OK] Project mi-tienda
```

Support:

```bash
orchest doctor --json
```

---

# 26. Logging

Use `tracing`.

Normal CLI output should remain concise.

Verbose diagnostics:

```bash
orchest --verbose service start nginx
```

Application logs:

```text
logs/orchest/orchest.log
```

Avoid leaking passwords or secrets into logs.

---

# 27. Testing

Create meaningful tests from the beginning.

Separate:

```text
unit tests
integration tests
platform tests
package installation tests
```

Do not require downloading hundreds of megabytes in normal CI tests.

Create small fixture archives that emulate packages.

Test:

- safe archive extraction,
- path traversal rejection,
- version resolution,
- manifest parsing,
- package install,
- package uninstall,
- configuration,
- process state,
- project runtime resolution.

---

# 28. First Implementation Scope

Do NOT attempt to implement every service immediately.

Build a vertical slice.

Milestone 1 should implement:

```text
1. Cargo workspace
2. configuration/root directory
3. package manifest parser
4. package download engine
5. ZIP extraction
6. TAR.GZ extraction
7. installed package registry
8. PHP package support
9. multiple PHP versions
10. global PHP version
11. project model
12. per-project PHP version
13. runtime resolver
14. orchest exec
15. basic process abstraction
16. JSON CLI output
17. structured errors
18. tests
```

The following command sequence must work by the end of milestone 1:

```bash
orchest init

orchest php install 8.4.15

orchest php install 8.5.10

orchest php list

orchest php default 8.5.10

orchest project add ./examples/legacy-app --name legacy-app

orchest project php legacy-app 8.4.15

orchest project show legacy-app

orchest exec php -v

orchest project exec legacy-app php -v

orchest doctor
```

Expected behavior:

```text
orchest exec php -v
```

uses PHP 8.5.10.

While:

```text
orchest project exec legacy-app php -v
```

uses PHP 8.4.15.

No system PHP should be required.

---

# 29. Development Rules

Follow these rules during implementation.

1. Compile frequently.

2. Do not leave large amounts of placeholder code.

3. Do not implement the React frontend yet.

4. Do not make Tauri a dependency of the domain/core crates.

5. Do not put business logic inside CLI command handlers.

6. Prefer explicit typed domain models.

7. Keep Windows and Linux equally important.

8. Avoid assumptions that only one version of a service can exist.

9. Avoid assumptions that only one instance of a service can exist.

10. Do not rely on the global PATH.

11. Managed binaries belong to Orchest.

12. Managed data belongs to Orchest.

13. Generated configs belong to Orchest.

14. Logs belong to Orchest.

15. Downloads should be reproducible from package manifests.

16. Package manifests must support URLs without checksums.

17. Never make checksums mandatory.

18. Validate archive structure and expected executables even without checksums.

19. Prefer atomic filesystem operations where possible.

20. Keep the architecture ready for a GUI without designing around the GUI.

---

# 30. Code Quality

Use idiomatic stable Rust.

Apply:

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo test --workspace
```

Keep modules reasonably small.

Document public core APIs.

Use dependency injection where it improves testability, especially for:

- downloader,
- filesystem,
- platform adapter,
- process runner.

Avoid enterprise-style abstraction for abstraction's sake.

---

# 31. README

Create a README explaining:

- what Orchest is;
- architecture;
- directory structure;
- package manifests;
- CLI examples;
- Windows support;
- Linux support;
- how runtime resolution works;
- how to add a new PHP version;
- how to add a new package;
- future API/Tauri architecture.

Include this conceptual architecture diagram:

```text
                 ┌─────────────────┐
                 │   orchest-cli   │
                 └────────┬────────┘
                          │
                          │
┌──────────────┐          │
│ Future Tauri │          │
│ React UI     │          │
└──────┬───────┘          │
       │                  │
       │           ┌──────▼───────┐
       └──────────►│ orchest-core │
                   └──────┬───────┘
                          │
             ┌────────────┼────────────┐
             │            │            │
             ▼            ▼            ▼
        packages       process      platform
             │            │            │
             └────────────┼────────────┘
                          │
                          ▼
                  Orchest managed
                     binaries
```

Later:

```text
React
  │
  ▼
Tauri / HTTP API
  │
  ▼
orchest-core
  │
  ├── packages
  ├── projects
  ├── services
  ├── processes
  └── platform
```

---

# 32. Start Now

Start by creating the Cargo workspace and implementing Milestone 1.

Do not implement the GUI.

First show the proposed workspace/file tree.

Then create the files.

Then implement each layer incrementally.

After each meaningful phase:

```bash
cargo check --workspace
cargo test --workspace
```

Fix compilation errors before continuing.

The goal of this first phase is to make Orchest genuinely useful from the command line before any graphical interface exists.


----

Decisiones que mantendría desde el principio
Hay una decisión particularmente importante: no modelaría PHP, MySQL, Nginx, etc. como casos completamente separados. Internamente conviene tener algo parecido a:
Package
├── Runtime
├── WebServer
├── Database
└── Service
Así instalar:
orchest php install 8.5.10
puede ser simplemente un alias cómodo de:
orchest package install php@8.5.10
Y después puedes llegar naturalmente a:
orchest nginx install 1.28.0
orchest mysql install 8.4.6
orchest node install 24.8.0
sin reescribir el sistema de descargas.
También evitaría instalar MySQL, Nginx, Redis, etc. mediante apt, winget, Chocolatey o similares. Eso destruiría buena parte de la ventaja de Orchest como orquestador autocontenido. La idea debe ser:
Internet
   ↓
Orchest package manifest
   ↓
Downloader
   ↓
C:\Orchest\bin\<package>\<version>
   ↓
Orchest Process Manager
y no:
Orchest
   ↓
apt / winget
   ↓
software global del SO
Sobre no usar checksums
Tu enfoque es viable, pero mantendría una pequeña protección adicional. No exigiría hashes, pero sí:
HTTPS
+
safe archive extraction
+
expected executable validation
+
staging directory
+
atomic install
Por ejemplo, al instalar PHP 8.5.10:
download
   ↓
cache/php-8.5.10.zip
   ↓
runtime/tmp/81d73f/
   ↓
comprobar php.exe
   ↓
bin/php/8.5.10/
Así el manifest solo necesita algo como:
url = "https://downloads.php.net/~windows/releases/archives/php-8.5.10-nts-Win32-vs17-x64.zip"
sin obligarte a mantener:
sha256 = "..."
Y una vez construida esta primera fase, el siguiente paso que haría no sería todavía React: sería diseñar el sistema de manifests de paquetes y servicios de Orchest para que PHP/Nginx/MySQL/MariaDB/MongoDB/Redis/Node puedan agregarse casi declarativamente. Ese componente va a determinar qué tan fácil será mantener Orchest a largo plazo.

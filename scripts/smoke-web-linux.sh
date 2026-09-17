#!/usr/bin/env bash
set -euo pipefail

: "${PHP84_ARCHIVE:?PHP84_ARCHIVE is required}"
: "${PHP85_ARCHIVE:?PHP85_ARCHIVE is required}"
: "${NGINX_ARCHIVE:?NGINX_ARCHIVE is required}"
: "${APACHE_ARCHIVE:?APACHE_ARCHIVE is required}"

root=$(mktemp -d)
chmod 755 "$root"
o() { orchest --root "$root" "$@"; }
cleanup() {
  if [[ "${DEBUG:-0}" == 1 ]]; then
    cat "$root/runtime/generated/nginx/logs/access.log" "$root/runtime/generated/nginx/logs/error.log" >&2 2>/dev/null || true
    o service status php@8.4.15 >&2 || true
    o service status php@8.5.10 >&2 || true
    cat "$root"/logs/services/php-web/*/stderr.log >&2 2>/dev/null || true
    ls -la "$root/www/direct" >&2
  fi
  o service stop nginx >/dev/null 2>&1 || true
}
trap cleanup EXIT

o init >/dev/null
o php install 8.4.15 --archive "$PHP84_ARCHIVE" >/dev/null
o php install 8.5.10 --archive "$PHP85_ARCHIVE" >/dev/null
o package install nginx@1.30.5 --archive "$NGINX_ARCHIVE" >/dev/null
o package install apache@2.4.68 --archive "$APACHE_ARCHIVE" >/dev/null
o config set ports.nginx_http 18080 >/dev/null
o config set ports.nginx_https 18443 >/dev/null
o config set ports.apache_http 18081 >/dev/null
o project add --name direct >/dev/null
o project add --name legacy >/dev/null
o project php direct 8.4.15 >/dev/null
o project php legacy 8.4.15 >/dev/null
o project web-server legacy apache >/dev/null
o project ssl direct on >/dev/null
o project ssl legacy on >/dev/null
printf '%s\n' '<?php echo "direct ", PHP_VERSION;' > "$root/www/direct/index.php"
printf '%s\n' '<?php echo "legacy ", PHP_VERSION;' > "$root/www/legacy/index.php"
o service start nginx >/dev/null
if [[ "${DEBUG:-0}" == 1 ]]; then
  o service config nginx >&2
  o service config apache >&2
  o service status apache >&2
fi

php="$root/bin/php/8.4.15/bin/php"
request() {
  "$php" -r '
    [$host, $scheme, $port, $root] = array_slice($argv, 1);
    $handle = curl_init("$scheme://$host:$port/index.php");
    curl_setopt($handle, CURLOPT_RETURNTRANSFER, true);
    curl_setopt($handle, CURLOPT_PROXY, "");
    curl_setopt($handle, CURLOPT_RESOLVE, ["$host:$port:127.0.0.1"]);
    if ($scheme === "https") curl_setopt($handle, CURLOPT_CAINFO, "$root/certificates/ca/cert.pem");
    $body = curl_exec($handle);
    if ($body === false || curl_getinfo($handle, CURLINFO_RESPONSE_CODE) !== 200) {
      fwrite(STDERR, curl_error($handle) . " HTTP " . curl_getinfo($handle, CURLINFO_RESPONSE_CODE) . " body=" . $body . "\n");
      exit(1);
    }
    echo $body;
  ' "$1" "$2" "$3" "$root"
}
for attempt in {1..30}; do
  if request direct.test http 18080 >/dev/null 2>&1; then break; fi
  sleep 0.2
done
[[ "$(request direct.test http 18080)" == "direct 8.4.15" ]]
[[ "$(request direct.test https 18443)" == "direct 8.4.15" ]]
[[ "$(request legacy.test http 18080)" == "legacy 8.4.15" ]]
[[ "$(request legacy.test https 18443)" == "legacy 8.4.15" ]]
if [[ "${CHECK_RENEWAL:-0}" == 1 ]]; then
  certificate="$root/certificates/sites/direct.test/cert.pem"
  touch -d '61 days ago' "$certificate"
  old_mtime=$(stat -c %Y "$certificate")
  for attempt in {1..30}; do
    if [[ "$(stat -c %Y "$certificate")" -gt "$old_mtime" ]]; then break; fi
    sleep 0.2
  done
  [[ "$(stat -c %Y "$certificate")" -gt "$old_mtime" ]]
  [[ "$(request direct.test https 18443)" == "direct 8.4.15" ]]
fi
o project php direct 8.5.10 >/dev/null
[[ "$(request direct.test http 18080)" == "direct 8.5.10" ]]
[[ "$(request legacy.test https 18443)" == "legacy 8.4.15" ]]
echo "Nginx, Apache, HTTPS, FastCGI and PHP reload passed"

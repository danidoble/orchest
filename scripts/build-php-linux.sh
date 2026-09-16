#!/usr/bin/env bash
set -euo pipefail

version="${PHP_VERSION:?PHP_VERSION is required}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Invalid PHP version: $version" >&2
  exit 2
fi

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
  build-essential pkg-config autoconf bison re2c ca-certificates curl \
  libxml2-dev libssl-dev libcurl4-openssl-dev libonig-dev \
  libsqlite3-dev libzip-dev zlib1g-dev patchelf python3

mkdir -p /build /out
cd /build
curl --fail --location --silent --show-error --proto '=https' --tlsv1.2 \
  "https://www.php.net/distributions/php-${version}.tar.gz" -o php.tar.gz
tar -xzf php.tar.gz
cd "php-${version}"

if ! ./configure \
  --prefix=/opt/orchest/php \
  --disable-all --enable-cli --enable-fpm --disable-cgi --without-pear \
  --with-openssl --with-curl --with-zlib --enable-mbstring \
  --with-libxml --enable-xml --enable-dom --enable-simplexml \
  --enable-xmlreader --enable-xmlwriter \
  --enable-pdo --with-pdo-sqlite --with-sqlite3 \
  --with-mysqli=mysqlnd --with-pdo-mysql=mysqlnd \
  --enable-sockets --enable-bcmath --enable-phar --with-zip \
  --enable-ctype --enable-filter --enable-tokenizer \
  --enable-fileinfo --enable-session > /build/configure.log 2>&1; then
  tail -100 /build/configure.log >&2
  exit 1
fi
if ! make -j "$(nproc)" > /build/make.log 2>&1; then
  tail -100 /build/make.log >&2
  exit 1
fi

package=/build/package
mkdir -p "$package/bin" "$package/sbin" "$package/lib"
install -m 0755 sapi/cli/php "$package/bin/php"
install -m 0755 sapi/fpm/php-fpm "$package/sbin/php-fpm"
install -m 0644 php.ini-development "$package/php.ini"

python3 /work/scripts/bundle-php-libs.py "$package"
patchelf --set-rpath '$ORIGIN/../lib' "$package/bin/php"
patchelf --set-rpath '$ORIGIN/../lib' "$package/sbin/php-fpm"

"$package/bin/php" -v
"$package/sbin/php-fpm" -v
tar -C "$package" -czf "/out/php-${version}-linux-x86_64.tar.gz" bin sbin lib php.ini

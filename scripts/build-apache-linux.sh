#!/usr/bin/env bash
set -euo pipefail

version="${APACHE_VERSION:?APACHE_VERSION is required}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || exit 2
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
  build-essential ca-certificates curl libapr1-dev libaprutil1-dev libpcre2-dev \
  libssl-dev zlib1g-dev libexpat1-dev patchelf python3
mkdir -p /build /out
cd /build
curl --fail --location --silent --show-error --proto '=https' --tlsv1.2 \
  "https://downloads.apache.org/httpd/httpd-${version}.tar.gz" -o httpd.tar.gz
tar -xzf httpd.tar.gz
cd "httpd-${version}"
./configure --prefix=/opt/orchest/apache --enable-so --with-mpm=event \
  --enable-mods-shared=most --enable-proxy --enable-proxy-http \
  --enable-proxy-fcgi --enable-rewrite --enable-headers > /build/configure.log 2>&1 || {
    tail -100 /build/configure.log >&2; exit 1;
  }
make -j "$(nproc)" > /build/make.log 2>&1 || {
  tail -100 /build/make.log >&2; exit 1;
}
make install DESTDIR=/build/stage > /build/install.log 2>&1 || {
  tail -100 /build/install.log >&2; exit 1;
}
source_root=/build/stage/opt/orchest/apache
package=/build/package
mkdir -p "$package/bin" "$package/modules" "$package/lib" "$package/conf"
install -m 0755 "$source_root/bin/httpd" "$package/bin/httpd"
cp -a "$source_root/modules/." "$package/modules/"
install -m 0644 "$source_root/conf/mime.types" "$package/conf/mime.types"
python3 /work/scripts/bundle-apache-libs.py "$package"
"$package/bin/httpd" -v
tar -C "$package" -czf "/out/apache-${version}-linux-x86_64.tar.gz" bin modules lib conf

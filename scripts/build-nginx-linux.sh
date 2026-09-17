#!/usr/bin/env bash
set -euo pipefail

version="${NGINX_VERSION:?NGINX_VERSION is required}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || exit 2
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends build-essential ca-certificates curl libpcre2-dev zlib1g-dev patchelf
mkdir -p /build /out
cd /build
curl --fail --location --silent --show-error --proto '=https' --tlsv1.2 \
  "https://nginx.org/download/nginx-${version}.tar.gz" -o nginx.tar.gz
tar -xzf nginx.tar.gz
cd "nginx-${version}"
./configure --prefix=/opt/orchest/nginx --sbin-path=/opt/orchest/nginx/sbin/nginx \
  --with-http_realip_module --with-http_stub_status_module \
  --without-http_gzip_module
make -j "$(nproc)"
package=/build/package
mkdir -p "$package/sbin" "$package/lib"
install -m 0755 objs/nginx "$package/sbin/nginx"
for library in $(ldd "$package/sbin/nginx" | awk '/libpcre2|libcrypt/ { print $3 }'); do
  cp -L "$library" "$package/lib/"
done
patchelf --set-rpath '$ORIGIN/../lib' "$package/sbin/nginx"
"$package/sbin/nginx" -v
tar -C "$package" -czf "/out/nginx-${version}-linux-x86_64.tar.gz" sbin lib

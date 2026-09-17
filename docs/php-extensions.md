# Extensiones PHP administradas

Orchest mantiene cada versión de PHP en un directorio aislado. `orchest php extensions VERSION` y `GET /api/v1/php/{version}/extensions` muestran los módulos que **realmente carga** esa instalación (`php -m`). Las dependencias de cada aplicación pueden pedir módulos adicionales.

| Uso | Módulos cubiertos por los artefactos nuevos |
| --- | --- |
| Núcleo habitual (`common`) | `bcmath`, `ctype`, `date`, `fileinfo`, `filter`, `hash`, `iconv`, `json`, `openssl`, `pcre`, `phar`, `session`, `sockets`, `tokenizer`, `zlib` |
| Texto y XML | `mbstring`, `dom`, `libxml`, `SimpleXML`, `xml`, `xmlreader`, `xmlwriter` |
| Imágenes y archivos | `gd`, `imagick`, `zip`, `bz2`, `exif` |
| HTTP e internacionalización | `curl`, `intl` |
| Bases de datos | `mysqli`, `mysqlnd`, `PDO`, `pdo_mysql`, `pdo_pgsql`, `pdo_sqlite`, `pgsql`, `sqlite3` |

`common` no es una extensión PHP con ese nombre: la primera fila enumera los módulos concretos que cubre esta configuración. `pdo_mysql`, `pdo_pgsql` y `pdo_sqlite` son controladores distintos; `PDO` por sí solo no conecta una base de datos.

| Framework | Módulos relevantes en esta integración |
| --- | --- |
| Laravel | `bcmath`, `ctype`, `fileinfo`, `mbstring`, `openssl`, `PDO` y el controlador de base de datos elegido; `gd` o `imagick` para funciones de imágenes. [Documentación de Laravel](https://laravel.com/framework/docs/images) |
| Symfony | `ctype`, `iconv`, `pcre`, `session`, `SimpleXML`, `tokenizer`; el proyecto puede añadir controladores de datos u otros módulos. [Requisitos de Symfony](https://symfony.com/doc/7.4/setup.html) |
| WordPress | `mysqli`, `curl`, `dom`, `exif`, `fileinfo`, `mbstring`, `zip`, `gd` e `imagick` para imágenes y funciones de mantenimiento. [Site Health de WordPress](https://developer.wordpress.org/reference/classes/wp_site_health/get_test_php_extensions/) |
| CakePHP | `mbstring`, `intl`, `PDO`, `SimpleXML` y el controlador PDO de la base elegida. [Requisitos de CakePHP](https://book.cakephp.org/5.x/installation.html) |

## Linux

La receta `scripts/build-php-linux.sh` compila los módulos anteriores dentro de PHP CLI y FPM. Incluye Imagick 3.8.1 desde PECL y comprueba los módulos con `php -m` antes de empaquetar. Los artefactos son específicos de la versión exacta de PHP; una nueva revisión del mismo PHP necesita una etiqueta de Release nueva y un cambio de URL en `manifests/php.toml`.

## Windows

Los ZIP oficiales NTS x64 de PHP traen las DLL de los módulos comunes. Orchest genera `conf.d/00-orchest-extensions.ini` para habilitar las que estén presentes y fija `extension_dir` a la ruta absoluta de la instalación. Imagick procede de los [ZIP NTS x64 de PECL 3.8.1](https://pecl.php.net/package/imagick/3.8.1/windows), uno por versión menor de PHP, con sus DLL de ImageMagick. `orchest php install VERSION` descarga el paquete de Imagick correspondiente cuando la instalación es en línea; para una instalación PHP con `--archive`, use después `orchest php imagick VERSION` o `orchest package install php-imagick@VERSION --archive ARCHIVO`.

Los módulos y sus dependencias deben confirmarse en la VM con `orchest php extensions VERSION` y con una petición web a un proyecto de esa versión. Los ZIP oficiales y los de PECL no deben mezclarse entre PHP 8.2, 8.3, 8.4 y 8.5.

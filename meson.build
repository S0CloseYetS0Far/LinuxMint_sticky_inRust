project('sticky', version : '1.33', meson_version : '>= 0.49.0')

i18n = import('i18n')

gettext_package = meson.project_name()

prefix = get_option('prefix')
bindir = get_option('bindir')
datadir = get_option('datadir')
sysconfdir = get_option('sysconfdir')

podir = meson.source_root() / 'po'

# The program itself is written in Rust and built with cargo. It is installed
# as /usr/lib/sticky/sticky and started by the usr/bin/sticky wrapper.
find_program('cargo')
cargo_build = find_program('meson/cargo-build.sh')

custom_target('sticky',
    output: 'sticky',
    command: [cargo_build, meson.source_root(), meson.current_build_dir(), '@OUTPUT@'],
    console: true,
    build_always_stale: true,
    build_by_default: true,
    install: true,
    install_dir: prefix / 'lib' / 'sticky',
)

subdir('data')
subdir('po')

install_subdir('etc', install_dir: sysconfdir, strip_directory: true)
install_subdir('usr', install_dir: prefix, strip_directory: true)

meson.add_install_script('meson/meson-postinstall.sh')

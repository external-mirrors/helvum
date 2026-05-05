project(
  'helvum',
  'rust',
  version: '0.6.0',
  license: 'GPL-3.0',
  meson_version: '>=1.9.0',
  default_options: [
    'warning_level=2',
    'buildtype=debugoptimized',
  ],
)

gnome = import('gnome')

base_id = 'org.pipewire.Helvum'

dependency('glib-2.0',     version: '>= 2.88.0')
dependency('gtk4',         version: '>= 4.22.0')
dependency('libadwaita-1', version: '>= 1.9.0')
dependency('libpipewire-0.3', version: '>= 1.6.0')

desktop_file_validate = find_program('desktop-file-validate', required: false)
appstream_util = find_program('appstream-util', required: false)
cargo = find_program('cargo', required: true)
sh = find_program('sh', required: true)

prefix = get_option('prefix')
bindir = prefix / get_option('bindir')
datadir = prefix / get_option('datadir')
iconsdir = datadir / 'icons'

meson.add_dist_script(
  'build-aux/dist-vendor.sh',
  meson.project_build_root() / 'meson-dist' / meson.project_name() + '-' + meson.project_version(),
  meson.project_source_root()
)

subdir('src')
subdir('data')

gnome.post_install(
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)

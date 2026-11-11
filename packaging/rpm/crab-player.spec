# RPM spec for Fedora, RHEL-compatible systems and openSUSE.
#
# Build: packaging/rpm/build.sh (in a Fedora container), or by hand with a
# source tarball in ~/rpmbuild/SOURCES and Rust 1.95+ on PATH:
#   rpmbuild -bb packaging/rpm/crab-player.spec
#
# Runtime requirements are given as sonames, so the same spec resolves to
# mpv-libs on Fedora and to libmpv2 on openSUSE. libmpv and the windowing
# and GL libraries are opened at runtime (dlopen), which the automatic
# dependency generator cannot see.

%global app_id io.github.mahziyarng98.CrabPlayer
# The release profile strips the binary; there is no debug info to package.
%global debug_package %{nil}

Name:           crab-player
Version:        0.1.0
Release:        1%{?dist}
Summary:        Fast native media player
License:        MIT
URL:            https://github.com/mahziyarng98/crab-player
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz
ExclusiveArch:  x86_64 aarch64

# Rust 1.95 or newer is required. It is not listed because distribution
# releases may ship an older one; packaging/rpm/build.sh uses rustup.
BuildRequires:  gcc
BuildRequires:  pkgconfig(mpv)
BuildRequires:  desktop-file-utils

Requires:       libmpv.so.2()(64bit)
Requires:       libEGL.so.1()(64bit)
Requires:       libGL.so.1()(64bit)
Requires:       libwayland-client.so.0()(64bit)
Requires:       libwayland-cursor.so.0()(64bit)
Requires:       libwayland-egl.so.1()(64bit)
Requires:       libxkbcommon.so.0()(64bit)
Requires:       libX11.so.6()(64bit)
Requires:       libXcursor.so.1()(64bit)
Requires:       libXi.so.6()(64bit)
Requires:       libXrandr.so.2()(64bit)
Requires:       libxkbcommon-x11.so.0()(64bit)
Requires:       hicolor-icon-theme
Recommends:     xdg-desktop-portal
# VA-API drivers for hardware decoding. Fedora: mesa-va-drivers for AMD
# (RPM Fusion's mesa-va-drivers-freeworld adds H.264/HEVC) and
# intel-media-driver from RPM Fusion for Intel. openSUSE: Mesa-libva for
# AMD and intel-media-driver for Intel.
Suggests:       (mesa-va-drivers or Mesa-libva)
Suggests:       intel-media-driver

%description
A light media player with a classic desktop layout: caption strip, video
canvas, transport bar and docked playlist.

Playback is provided by libmpv, with VA-API/NVDEC hardware decoding,
subtitle rendering and network streams. Every format the installed FFmpeg
decodes is supported: on Fedora install ffmpeg-libs from RPM Fusion and on
openSUSE the Packman libav* packages for patented codecs such as H.264 and
HEVC. The interface runs natively on Wayland and X11.

%prep
%autosetup -n %{name}-%{version}

%build
cargo build --release --locked

%check
cargo test --release --locked
desktop-file-validate packaging/%{app_id}.desktop

%install
install -Dm755 target/release/%{name} %{buildroot}%{_bindir}/%{name}
install -Dm644 packaging/%{app_id}.desktop %{buildroot}%{_datadir}/applications/%{app_id}.desktop
install -Dm644 packaging/%{app_id}.metainfo.xml %{buildroot}%{_metainfodir}/%{app_id}.metainfo.xml
for size in 16 24 32 48 64 128 256 512; do
  install -Dm644 assets/icons/hicolor/${size}x${size}/apps/%{app_id}.png \
    %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/%{app_id}.png
done

%files
%license LICENSE
%doc README.md
%{_bindir}/%{name}
%{_datadir}/applications/%{app_id}.desktop
%{_metainfodir}/%{app_id}.metainfo.xml
%{_datadir}/icons/hicolor/*/apps/%{app_id}.png

%changelog
* Wed Nov 11 2026 Mahziyar <289230375+mahziyarng98@users.noreply.github.com> - 0.1.0-1
- First release.

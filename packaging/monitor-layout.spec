# RPM spec for Monitor Layout.
#
# Local build (no network in %%build thanks to `cargo vendor`; see
# packaging/build-rpm.sh which prepares the vendor tarball):
#   ./packaging/build-rpm.sh
#
# The application talks to org.gnome.Mutter.DisplayConfig on the session bus
# — an unrestricted user-session interface, so no extra permissions or
# polkit rules are needed and the app never requires root.

Name:           monitor-layout
Version:        0.1.0
Release:        1%{?dist}
Summary:        Visual display layout manager for GNOME with partial mirroring
License:        GPL-3.0-or-later
URL:            https://github.com/christosdaggas/Gnome-Monitors
Source0:        %{name}-%{version}.tar.gz
# Created with: cargo vendor (see packaging/build-rpm.sh)
Source1:        %{name}-%{version}-vendor.tar.xz

BuildRequires:  rust >= 1.92
BuildRequires:  cargo
BuildRequires:  gcc
BuildRequires:  pkgconfig(gtk4) >= 4.14
BuildRequires:  pkgconfig(libadwaita-1) >= 1.5
BuildRequires:  desktop-file-utils
BuildRequires:  appstream

# Runtime floors match the newest APIs actually called: GdkMonitor.connector
# (GTK 4.10) and AdwStyleManager.accent_color_rgba (libadwaita 1.6).
Requires:       gtk4 >= 4.10
Requires:       libadwaita >= 1.6
# The backend requires a running GNOME (Mutter) Wayland session at runtime.

%description
Monitor Layout is a visual display manager for GNOME on Wayland. It talks
directly to Mutter's DisplayConfig D-Bus interface and supports partial
mirroring: two displays can form one mirror group while a third remains an
independent extended display — ideal for KVM-over-IP setups. Includes
drag-and-drop arrangement, per-display settings, EDID-identity-based
profiles, and a safe apply flow with automatic revert.

%prep
%autosetup
mkdir -p .cargo vendor
tar -xJf %{SOURCE1}
cat > .cargo/config.toml <<EOF
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF

%build
cargo build --release --offline --locked --workspace

%install
install -Dm755 target/release/monitor-layout %{buildroot}%{_bindir}/monitor-layout
install -Dm755 target/release/monitor-layout-revert-helper %{buildroot}%{_bindir}/monitor-layout-revert-helper
install -Dm755 target/release/monitor-layout-ctl %{buildroot}%{_bindir}/monitor-layout-ctl
install -Dm644 data/desktop/com.chrisdaggas.MonitorLayout.desktop \
    %{buildroot}%{_datadir}/applications/com.chrisdaggas.MonitorLayout.desktop
install -Dm644 data/metainfo/com.chrisdaggas.MonitorLayout.metainfo.xml \
    %{buildroot}%{_metainfodir}/com.chrisdaggas.MonitorLayout.metainfo.xml
install -Dm644 data/icons/hicolor/scalable/apps/com.chrisdaggas.MonitorLayout.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/com.chrisdaggas.MonitorLayout.svg

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/com.chrisdaggas.MonitorLayout.desktop
appstreamcli validate --no-net %{buildroot}%{_metainfodir}/com.chrisdaggas.MonitorLayout.metainfo.xml

%files
%license LICENSE
%doc README.md
%{_bindir}/monitor-layout
%{_bindir}/monitor-layout-revert-helper
%{_bindir}/monitor-layout-ctl
%{_datadir}/applications/com.chrisdaggas.MonitorLayout.desktop
%{_metainfodir}/com.chrisdaggas.MonitorLayout.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/com.chrisdaggas.MonitorLayout.svg

%changelog
* Sun Jul 19 2026 Christos A. Daggas <christos@daggas.gr> - 0.1.0-1
- Initial package

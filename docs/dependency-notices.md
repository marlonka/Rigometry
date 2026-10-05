# Dependency license inventory

Generated 2026-10-04 from the pinned Cargo.lock using:

```powershell
cargo metadata --locked --offline --format-version 1 --filter-platform x86_64-pc-windows-msvc
```

202 resolved dependency package versions for Windows x86-64, including build/test dependencies. This is a Cargo metadata inventory; it is not a claim that every resolved package is linked into the release executable. All reported packages have a license expression.

Application code: MIT. For dual-licensed dependencies use a permissive offered branch, such as MIT or Apache-2.0. In particular, self_cell offers Apache-2.0 OR GPL-2.0-only; choose Apache-2.0. License expressions with AND require all listed components, including embedded-font and Unicode notices.

The Rust standard library is outside Cargo.lock; its [original notices](toolchain-notices/rust-1.99.0/COPYRIGHT-library.html) are included separately. The statically linked Microsoft runtime retains its Microsoft terms; the application MIT license does not replace them.

## Distribution notes

- Include the application LICENSE, this inventory, and bundled third-party license texts with the portable package.
- Default egui fonts are separate assets: Hack (MIT/Bitstream Vera terms), Noto Emoji (OFL), Ubuntu (Ubuntu Font Licence), and emoji-icon-font (MIT). Preserve the font notices even when system fonts override the defaults.
- ICU and Unicode data use Unicode-3.0 notices; unicode-ident also includes Unicode terms alongside its Rust-code license.
- Cargo metadata licenses do not cover separately loaded installed NVIDIA/AMD/Intel drivers. No vendor driver binary is bundled. The NVML ABI notice is included in third-party-licenses.txt separately from Rust package notices.
- Original license/notice texts from cached crates and exact upstream commits are collected in third-party-licenses.txt. Identical text is included once with all associated packages. Upstream supplements cover workspace-published crates whose crates.io archive omits root license files; font and generated-binding notices are retained.

## Pinned packages

| Package | Version | Relationship | Declared license | Packaged notice files |
| --- | --- | --- | --- | --- |
| [accesskit](https://crates.io/crates/accesskit/0.24.1) | 0.24.1 | Transitive | MIT OR Apache-2.0 | upstream license supplement |
| [accesskit_consumer](https://crates.io/crates/accesskit_consumer/0.35.0) | 0.35.0 | Transitive | MIT OR Apache-2.0 | upstream license supplement |
| [accesskit_windows](https://crates.io/crates/accesskit_windows/0.32.1) | 0.32.1 | Transitive | MIT OR Apache-2.0 | upstream license supplement |
| [accesskit_winit](https://crates.io/crates/accesskit_winit/0.32.2) | 0.32.2 | Transitive | Apache-2.0 | upstream license supplement |
| [adler2](https://crates.io/crates/adler2/2.0.1) | 2.0.1 | Transitive | 0BSD OR MIT OR Apache-2.0 | LICENSE-0BSD, LICENSE-APACHE, LICENSE-MIT |
| [ahash](https://crates.io/crates/ahash/0.8.12) | 0.8.12 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [arboard](https://crates.io/crates/arboard/3.6.1) | 3.6.1 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE.txt, LICENSE-MIT.txt |
| [arrayvec](https://crates.io/crates/arrayvec/0.7.8) | 0.7.8 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [autocfg](https://crates.io/crates/autocfg/1.5.1) | 1.5.1 | Transitive | Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT |
| [bitflags](https://crates.io/crates/bitflags/2.13.2) | 2.13.2 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [bytemuck](https://crates.io/crates/bytemuck/1.25.2) | 1.25.2 | Transitive | Zlib OR Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT, LICENSE-ZLIB |
| [bytemuck_derive](https://crates.io/crates/bytemuck_derive/1.12.1) | 1.12.1 | Transitive | Zlib OR Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT, LICENSE-ZLIB |
| [byteorder-lite](https://crates.io/crates/byteorder-lite/0.1.0) | 0.1.0 | Transitive | Unlicense OR MIT | LICENSE-MIT |
| [cfg-if](https://crates.io/crates/cfg-if/1.0.5) | 1.0.5 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [cfg_aliases](https://crates.io/crates/cfg_aliases/0.2.2) | 0.2.2 | Transitive | MIT | LICENSE, NOTICES.md |
| [clipboard-win](https://crates.io/crates/clipboard-win/5.4.1) | 5.4.1 | Transitive | BSL-1.0 | upstream license supplement |
| [color](https://crates.io/crates/color/0.3.3) | 0.3.3 | Transitive | Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT |
| [crc32fast](https://crates.io/crates/crc32fast/1.5.2) | 1.5.2 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [crossbeam-channel](https://crates.io/crates/crossbeam-channel/0.5.15) | 0.5.15 | Direct | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT, LICENSE-THIRD-PARTY |
| [crossbeam-utils](https://crates.io/crates/crossbeam-utils/0.8.23) | 0.8.23 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [csv](https://crates.io/crates/csv/1.4.0) | 1.4.0 | Direct | Unlicense/MIT | COPYING, LICENSE-MIT |
| [csv-core](https://crates.io/crates/csv-core/0.1.13) | 0.1.13 | Transitive | Unlicense/MIT | COPYING, LICENSE-MIT |
| [cursor-icon](https://crates.io/crates/cursor-icon/1.2.0) | 1.2.0 | Transitive | MIT OR Apache-2.0 OR Zlib | LICENSE-APACHE, LICENSE-MIT, LICENSE-ZLIB |
| [displaydoc](https://crates.io/crates/displaydoc/0.2.7) | 0.2.7 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [document-features](https://crates.io/crates/document-features/0.2.12) | 0.2.12 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [dpi](https://crates.io/crates/dpi/0.1.2) | 0.1.2 | Transitive | Apache-2.0 AND MIT | LICENSE, LICENSE-LIBM-MIT |
| [ecolor](https://crates.io/crates/ecolor/0.34.3) | 0.34.3 | Transitive | MIT OR Apache-2.0 | upstream license supplement |
| [eframe](https://crates.io/crates/eframe/0.34.3) | 0.34.3 | Direct | MIT OR Apache-2.0 | upstream license supplement |
| [egui](https://crates.io/crates/egui/0.34.3) | 0.34.3 | Transitive | MIT OR Apache-2.0 | upstream license supplement |
| [egui-winit](https://crates.io/crates/egui-winit/0.34.3) | 0.34.3 | Transitive | MIT OR Apache-2.0 | upstream license supplement |
| [egui_extras](https://crates.io/crates/egui_extras/0.34.3) | 0.34.3 | Direct | MIT OR Apache-2.0 | upstream license supplement |
| [egui_glow](https://crates.io/crates/egui_glow/0.34.3) | 0.34.3 | Transitive | MIT OR Apache-2.0 | upstream license supplement |
| [egui_kittest](https://crates.io/crates/egui_kittest/0.34.3) | 0.34.3 | Direct | MIT OR Apache-2.0 | upstream license supplement |
| [emath](https://crates.io/crates/emath/0.34.3) | 0.34.3 | Transitive | MIT OR Apache-2.0 | upstream license supplement |
| [enum-map](https://crates.io/crates/enum-map/2.7.3) | 2.7.3 | Transitive | MIT OR Apache-2.0 | upstream license supplement |
| [enum-map-derive](https://crates.io/crates/enum-map-derive/0.17.0) | 0.17.0 | Transitive | MIT OR Apache-2.0 | upstream license supplement |
| [enumn](https://crates.io/crates/enumn/0.1.14) | 0.1.14 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [epaint](https://crates.io/crates/epaint/0.34.3) | 0.34.3 | Transitive | MIT OR Apache-2.0 | upstream license supplement |
| [epaint_default_fonts](https://crates.io/crates/epaint_default_fonts/0.34.3) | 0.34.3 | Transitive | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | fonts/emoji-icon-font-mit-license.txt, fonts/Hack-Regular.txt, fonts/OFL.txt, fonts/UFL.txt, upstream license supplement |
| [error-code](https://crates.io/crates/error-code/3.4.0) | 3.4.0 | Transitive | BSL-1.0 | LICENSE |
| [euclid](https://crates.io/crates/euclid/0.22.14) | 0.22.14 | Transitive | MIT OR Apache-2.0 | COPYRIGHT, LICENSE-APACHE, LICENSE-MIT |
| [fastrand](https://crates.io/crates/fastrand/2.5.0) | 2.5.0 | Transitive | Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT |
| [fax](https://crates.io/crates/fax/0.2.7) | 0.2.7 | Transitive | MIT | LICENSE |
| [fdeflate](https://crates.io/crates/fdeflate/0.3.7) | 0.3.7 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [fearless_simd](https://crates.io/crates/fearless_simd/0.3.0) | 0.3.0 | Transitive | Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT |
| [flate2](https://crates.io/crates/flate2/1.1.10) | 1.1.10 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [foldhash](https://crates.io/crates/foldhash/0.2.0) | 0.2.0 | Transitive | Zlib | LICENSE |
| [font-types](https://crates.io/crates/font-types/0.11.3) | 0.11.3 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [form_urlencoded](https://crates.io/crates/form_urlencoded/1.2.2) | 1.2.2 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [getrandom](https://crates.io/crates/getrandom/0.3.4) | 0.3.4 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [gl_generator](https://crates.io/crates/gl_generator/0.14.0) | 0.14.0 | Transitive | Apache-2.0 | upstream license supplement |
| [glow](https://crates.io/crates/glow/0.17.0) | 0.17.0 | Transitive | MIT OR Apache-2.0 OR Zlib | LICENSE-APACHE, LICENSE-MIT, LICENSE-ZLIB |
| [glutin](https://crates.io/crates/glutin/0.32.3) | 0.32.3 | Transitive | Apache-2.0 | LICENSE |
| [glutin-winit](https://crates.io/crates/glutin-winit/0.5.0) | 0.5.0 | Transitive | MIT | LICENSE |
| [glutin_egl_sys](https://crates.io/crates/glutin_egl_sys/0.7.1) | 0.7.1 | Transitive | Apache-2.0 | LICENSE |
| [glutin_wgl_sys](https://crates.io/crates/glutin_wgl_sys/0.6.1) | 0.6.1 | Transitive | Apache-2.0 | LICENSE |
| [half](https://crates.io/crates/half/2.7.1) | 2.7.1 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [hashbrown](https://crates.io/crates/hashbrown/0.16.1) | 0.16.1 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [home](https://crates.io/crates/home/0.5.12) | 0.5.12 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [icu_collections](https://crates.io/crates/icu_collections/2.3.0) | 2.3.0 | Transitive | Unicode-3.0 | LICENSE |
| [icu_locale_core](https://crates.io/crates/icu_locale_core/2.3.0) | 2.3.0 | Transitive | Unicode-3.0 | LICENSE |
| [icu_normalizer](https://crates.io/crates/icu_normalizer/2.3.0) | 2.3.0 | Transitive | Unicode-3.0 | LICENSE |
| [icu_normalizer_data](https://crates.io/crates/icu_normalizer_data/2.3.0) | 2.3.0 | Transitive | Unicode-3.0 | LICENSE |
| [icu_properties](https://crates.io/crates/icu_properties/2.3.0) | 2.3.0 | Transitive | Unicode-3.0 | LICENSE |
| [icu_properties_data](https://crates.io/crates/icu_properties_data/2.3.0) | 2.3.0 | Transitive | Unicode-3.0 | LICENSE |
| [icu_provider](https://crates.io/crates/icu_provider/2.3.1) | 2.3.1 | Transitive | Unicode-3.0 | LICENSE |
| [idna](https://crates.io/crates/idna/1.1.0) | 1.1.0 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [idna_adapter](https://crates.io/crates/idna_adapter/1.2.2) | 1.2.2 | Transitive | Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT |
| [image](https://crates.io/crates/image/0.25.10) | 0.25.10 | Direct | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [itoa](https://crates.io/crates/itoa/1.0.18) | 1.0.18 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [khronos_api](https://crates.io/crates/khronos_api/3.1.0) | 3.1.0 | Transitive | Apache-2.0 | upstream license supplement |
| [kittest](https://crates.io/crates/kittest/0.4.0) | 0.4.0 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [kurbo](https://crates.io/crates/kurbo/0.13.1) | 0.13.1 | Transitive | Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT |
| [libc](https://crates.io/crates/libc/0.2.190) | 0.2.190 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [libloading](https://crates.io/crates/libloading/0.8.9) | 0.8.9 | Direct | ISC | LICENSE |
| [linebender_resource_handle](https://crates.io/crates/linebender_resource_handle/0.1.1) | 0.1.1 | Transitive | Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT |
| [litemap](https://crates.io/crates/litemap/0.8.3) | 0.8.3 | Transitive | Unicode-3.0 | LICENSE |
| [litrs](https://crates.io/crates/litrs/1.0.0) | 1.0.0 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [lock_api](https://crates.io/crates/lock_api/0.4.14) | 0.4.14 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [log](https://crates.io/crates/log/0.4.34) | 0.4.34 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [memchr](https://crates.io/crates/memchr/2.8.3) | 2.8.3 | Transitive | Unlicense OR MIT | COPYING, LICENSE-MIT |
| [memoffset](https://crates.io/crates/memoffset/0.9.1) | 0.9.1 | Transitive | MIT | LICENSE |
| [mime](https://crates.io/crates/mime/0.3.17) | 0.3.17 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [mime_guess2](https://crates.io/crates/mime_guess2/2.3.1) | 2.3.1 | Transitive | MIT | LICENSE |
| [miniz_oxide](https://crates.io/crates/miniz_oxide/0.8.9) | 0.8.9 | Transitive | MIT OR Zlib OR Apache-2.0 | LICENSE, LICENSE-APACHE.md, LICENSE-MIT.md, LICENSE-ZLIB.md |
| [miniz_oxide](https://crates.io/crates/miniz_oxide/0.9.1) | 0.9.1 | Transitive | MIT OR Zlib OR Apache-2.0 | LICENSE, LICENSE-APACHE.md, LICENSE-MIT.md, LICENSE-ZLIB.md |
| [moxcms](https://crates.io/crates/moxcms/0.8.1) | 0.8.1 | Transitive | BSD-3-Clause OR Apache-2.0 | LICENSE-APACHE.md, LICENSE.md |
| [nohash-hasher](https://crates.io/crates/nohash-hasher/0.2.0) | 0.2.0 | Transitive | Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT |
| [ntapi](https://crates.io/crates/ntapi/0.4.3) | 0.4.3 | Transitive | Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT |
| [num-traits](https://crates.io/crates/num-traits/0.2.19) | 0.2.19 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [once_cell](https://crates.io/crates/once_cell/1.21.4) | 1.21.4 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [parking_lot](https://crates.io/crates/parking_lot/0.12.5) | 0.12.5 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [parking_lot_core](https://crates.io/crates/parking_lot_core/0.9.12) | 0.9.12 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [peniko](https://crates.io/crates/peniko/0.6.1) | 0.6.1 | Transitive | Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT |
| [percent-encoding](https://crates.io/crates/percent-encoding/2.3.2) | 2.3.2 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [phf](https://crates.io/crates/phf/0.11.3) | 0.11.3 | Transitive | MIT | LICENSE |
| [phf_generator](https://crates.io/crates/phf_generator/0.11.3) | 0.11.3 | Transitive | MIT | LICENSE |
| [phf_macros](https://crates.io/crates/phf_macros/0.11.3) | 0.11.3 | Transitive | MIT | LICENSE |
| [phf_shared](https://crates.io/crates/phf_shared/0.11.3) | 0.11.3 | Transitive | MIT | LICENSE |
| [pin-project-lite](https://crates.io/crates/pin-project-lite/0.2.17) | 0.2.17 | Transitive | Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT |
| [png](https://crates.io/crates/png/0.18.1) | 0.18.1 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [polycool](https://crates.io/crates/polycool/0.4.0) | 0.4.0 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [potential_utf](https://crates.io/crates/potential_utf/0.1.6) | 0.1.6 | Transitive | Unicode-3.0 | LICENSE |
| [proc-macro2](https://crates.io/crates/proc-macro2/1.0.107) | 1.0.107 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [profiling](https://crates.io/crates/profiling/1.0.18) | 1.0.18 | Transitive | MIT OR Apache-2.0 | upstream license supplement |
| [pxfm](https://crates.io/crates/pxfm/0.1.30) | 0.1.30 | Transitive | BSD-3-Clause OR Apache-2.0 | LICENSE-APACHE.md, LICENSE.md |
| [quick-error](https://crates.io/crates/quick-error/2.0.1) | 2.0.1 | Transitive | MIT/Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [quote](https://crates.io/crates/quote/1.0.47) | 1.0.47 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [rand](https://crates.io/crates/rand/0.8.8) | 0.8.8 | Transitive | MIT OR Apache-2.0 | COPYRIGHT, LICENSE-APACHE, LICENSE-MIT |
| [rand_core](https://crates.io/crates/rand_core/0.6.4) | 0.6.4 | Transitive | MIT OR Apache-2.0 | COPYRIGHT, LICENSE-APACHE, LICENSE-MIT |
| [raw-cpuid](https://crates.io/crates/raw-cpuid/11.6.0) | 11.6.0 | Direct | MIT | LICENSE.md |
| [raw-window-handle](https://crates.io/crates/raw-window-handle/0.6.2) | 0.6.2 | Transitive | MIT OR Apache-2.0 OR Zlib | LICENSE-APACHE.md, LICENSE-MIT.md, LICENSE-ZLIB.md |
| [read-fonts](https://crates.io/crates/read-fonts/0.37.0) | 0.37.0 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [rfd](https://crates.io/crates/rfd/0.15.4) | 0.15.4 | Direct | MIT | LICENSE |
| [ron](https://crates.io/crates/ron/0.12.2) | 0.12.2 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [ryu](https://crates.io/crates/ryu/1.0.23) | 1.0.23 | Transitive | Apache-2.0 OR BSL-1.0 | LICENSE-APACHE, LICENSE-BOOST |
| [scopeguard](https://crates.io/crates/scopeguard/1.2.0) | 1.2.0 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [self_cell](https://crates.io/crates/self_cell/1.3.0) | 1.3.0 | Transitive | Apache-2.0 OR GPL-2.0-only | LICENSE-APACHE, LICENSE-GPLv2 |
| [serde](https://crates.io/crates/serde/1.0.228) | 1.0.228 | Direct | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [serde_core](https://crates.io/crates/serde_core/1.0.228) | 1.0.228 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [serde_derive](https://crates.io/crates/serde_derive/1.0.228) | 1.0.228 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [serde_json](https://crates.io/crates/serde_json/1.0.150) | 1.0.150 | Direct | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [serde_spanned](https://crates.io/crates/serde_spanned/1.1.1) | 1.1.1 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [simd-adler32](https://crates.io/crates/simd-adler32/0.3.10) | 0.3.10 | Transitive | MIT | LICENSE.md |
| [siphasher](https://crates.io/crates/siphasher/1.0.4) | 1.0.4 | Transitive | MIT OR Apache-2.0 | COPYING, LICENSE-APACHE, LICENSE-MIT |
| [skrifa](https://crates.io/crates/skrifa/0.40.0) | 0.40.0 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [smallvec](https://crates.io/crates/smallvec/1.16.2) | 1.16.2 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [smol_str](https://crates.io/crates/smol_str/0.2.2) | 0.2.2 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [stable_deref_trait](https://crates.io/crates/stable_deref_trait/1.2.1) | 1.2.1 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [static_assertions](https://crates.io/crates/static_assertions/1.1.0) | 1.1.0 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [syn](https://crates.io/crates/syn/2.0.119) | 2.0.119 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [syn](https://crates.io/crates/syn/3.0.6) | 3.0.6 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [synstructure](https://crates.io/crates/synstructure/0.14.0) | 0.14.0 | Transitive | MIT | LICENSE |
| [sysinfo](https://crates.io/crates/sysinfo/0.37.2) | 0.37.2 | Direct | MIT | LICENSE |
| [tempfile](https://crates.io/crates/tempfile/3.23.0) | 3.23.0 | Direct | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [tiff](https://crates.io/crates/tiff/0.11.3) | 0.11.3 | Transitive | MIT | LICENSE, tests/COPYRIGHT |
| [tinystr](https://crates.io/crates/tinystr/0.8.4) | 0.8.4 | Transitive | Unicode-3.0 | LICENSE |
| [toml](https://crates.io/crates/toml/1.1.6+spec-1.1.0) | 1.1.6+spec-1.1.0 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [toml_datetime](https://crates.io/crates/toml_datetime/1.1.1+spec-1.1.0) | 1.1.1+spec-1.1.0 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [toml_parser](https://crates.io/crates/toml_parser/1.1.3+spec-1.1.0) | 1.1.3+spec-1.1.0 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [tracing](https://crates.io/crates/tracing/0.1.44) | 0.1.44 | Transitive | MIT | LICENSE |
| [tracing-attributes](https://crates.io/crates/tracing-attributes/0.1.31) | 0.1.31 | Transitive | MIT | LICENSE |
| [tracing-core](https://crates.io/crates/tracing-core/0.1.36) | 0.1.36 | Transitive | MIT | LICENSE, src/spin/LICENSE |
| [typeid](https://crates.io/crates/typeid/1.0.3) | 1.0.3 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [unicase](https://crates.io/crates/unicase/2.9.0) | 2.9.0 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [unicode-ident](https://crates.io/crates/unicode-ident/1.0.26) | 1.0.26 | Transitive | (MIT OR Apache-2.0) AND Unicode-3.0 | LICENSE-APACHE, LICENSE-MIT, LICENSE-UNICODE |
| [unicode-segmentation](https://crates.io/crates/unicode-segmentation/1.13.3) | 1.13.3 | Transitive | MIT OR Apache-2.0 | COPYRIGHT, LICENSE-APACHE, LICENSE-MIT |
| [url](https://crates.io/crates/url/2.5.8) | 2.5.8 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [utf8_iter](https://crates.io/crates/utf8_iter/1.0.4) | 1.0.4 | Transitive | Apache-2.0 OR MIT | COPYRIGHT, LICENSE-APACHE, LICENSE-MIT |
| [uuid](https://crates.io/crates/uuid/1.27.0) | 1.27.0 | Transitive | Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT |
| [vello_common](https://crates.io/crates/vello_common/0.0.6) | 0.0.6 | Transitive | Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT |
| [vello_cpu](https://crates.io/crates/vello_cpu/0.0.6) | 0.0.6 | Transitive | Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-MIT |
| [version_check](https://crates.io/crates/version_check/0.9.5) | 0.9.5 | Transitive | MIT/Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [web-time](https://crates.io/crates/web-time/1.1.0) | 1.1.0 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [webbrowser](https://crates.io/crates/webbrowser/1.2.4) | 1.2.4 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [weezl](https://crates.io/crates/weezl/0.1.12) | 0.1.12 | Transitive | MIT OR Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [winapi](https://crates.io/crates/winapi/0.3.9) | 0.3.9 | Transitive | MIT/Apache-2.0 | LICENSE-APACHE, LICENSE-MIT |
| [windows](https://crates.io/crates/windows/0.61.3) | 0.61.3 | Direct | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows](https://crates.io/crates/windows/0.62.2) | 0.62.2 | Direct | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-collections](https://crates.io/crates/windows-collections/0.2.0) | 0.2.0 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-collections](https://crates.io/crates/windows-collections/0.3.2) | 0.3.2 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-core](https://crates.io/crates/windows-core/0.61.2) | 0.61.2 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-core](https://crates.io/crates/windows-core/0.62.2) | 0.62.2 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-future](https://crates.io/crates/windows-future/0.2.1) | 0.2.1 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-future](https://crates.io/crates/windows-future/0.3.2) | 0.3.2 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-implement](https://crates.io/crates/windows-implement/0.60.2) | 0.60.2 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-interface](https://crates.io/crates/windows-interface/0.59.3) | 0.59.3 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-link](https://crates.io/crates/windows-link/0.1.3) | 0.1.3 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-link](https://crates.io/crates/windows-link/0.2.1) | 0.2.1 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-numerics](https://crates.io/crates/windows-numerics/0.2.0) | 0.2.0 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-numerics](https://crates.io/crates/windows-numerics/0.3.1) | 0.3.1 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-result](https://crates.io/crates/windows-result/0.3.4) | 0.3.4 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-result](https://crates.io/crates/windows-result/0.4.1) | 0.4.1 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-strings](https://crates.io/crates/windows-strings/0.4.2) | 0.4.2 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-strings](https://crates.io/crates/windows-strings/0.5.1) | 0.5.1 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-sys](https://crates.io/crates/windows-sys/0.52.0) | 0.52.0 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-sys](https://crates.io/crates/windows-sys/0.59.0) | 0.59.0 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-sys](https://crates.io/crates/windows-sys/0.60.2) | 0.60.2 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-sys](https://crates.io/crates/windows-sys/0.61.2) | 0.61.2 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-targets](https://crates.io/crates/windows-targets/0.52.6) | 0.52.6 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-targets](https://crates.io/crates/windows-targets/0.53.5) | 0.53.5 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-threading](https://crates.io/crates/windows-threading/0.1.0) | 0.1.0 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows-threading](https://crates.io/crates/windows-threading/0.2.1) | 0.2.1 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows_x86_64_msvc](https://crates.io/crates/windows_x86_64_msvc/0.52.6) | 0.52.6 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [windows_x86_64_msvc](https://crates.io/crates/windows_x86_64_msvc/0.53.1) | 0.53.1 | Transitive | MIT OR Apache-2.0 | license-apache-2.0, license-mit |
| [winit](https://crates.io/crates/winit/0.30.13) | 0.30.13 | Transitive | Apache-2.0 | LICENSE |
| [winnow](https://crates.io/crates/winnow/1.0.4) | 1.0.4 | Transitive | MIT | LICENSE-MIT |
| [writeable](https://crates.io/crates/writeable/0.6.4) | 0.6.4 | Transitive | Unicode-3.0 | LICENSE |
| [xml-rs](https://crates.io/crates/xml-rs/0.8.29) | 0.8.29 | Transitive | MIT | LICENSE |
| [yoke](https://crates.io/crates/yoke/0.8.3) | 0.8.3 | Transitive | Unicode-3.0 | LICENSE |
| [yoke-derive](https://crates.io/crates/yoke-derive/0.8.4) | 0.8.4 | Transitive | Unicode-3.0 | LICENSE |
| [zerocopy](https://crates.io/crates/zerocopy/0.8.59) | 0.8.59 | Transitive | BSD-2-Clause OR Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-BSD, LICENSE-MIT |
| [zerocopy-derive](https://crates.io/crates/zerocopy-derive/0.8.59) | 0.8.59 | Transitive | BSD-2-Clause OR Apache-2.0 OR MIT | LICENSE-APACHE, LICENSE-BSD, LICENSE-MIT |
| [zerofrom](https://crates.io/crates/zerofrom/0.1.8) | 0.1.8 | Transitive | Unicode-3.0 | LICENSE |
| [zerofrom-derive](https://crates.io/crates/zerofrom-derive/0.1.8) | 0.1.8 | Transitive | Unicode-3.0 | LICENSE |
| [zerotrie](https://crates.io/crates/zerotrie/0.2.5) | 0.2.5 | Transitive | Unicode-3.0 | LICENSE |
| [zerovec](https://crates.io/crates/zerovec/0.11.8) | 0.11.8 | Transitive | Unicode-3.0 | LICENSE |
| [zerovec-derive](https://crates.io/crates/zerovec-derive/0.11.6) | 0.11.6 | Transitive | Unicode-3.0 | LICENSE |
| [zlib-rs](https://crates.io/crates/zlib-rs/0.6.8) | 0.6.8 | Transitive | Zlib | LICENSE |
| [zmij](https://crates.io/crates/zmij/1.0.23) | 1.0.23 | Transitive | MIT | LICENSE-MIT |
| [zune-core](https://crates.io/crates/zune-core/0.5.3) | 0.5.3 | Transitive | MIT OR Apache-2.0 OR Zlib | LICENSE-APACHE, LICENSE-MIT, LICENSE-ZLIB |
| [zune-jpeg](https://crates.io/crates/zune-jpeg/0.5.15) | 0.5.15 | Transitive | MIT OR Apache-2.0 OR Zlib | LICENSE-APACHE, LICENSE-MIT, LICENSE-ZLIB |

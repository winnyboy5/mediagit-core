# Third-Party Licenses

MediaGit itself is licensed under the [Business Source License 1.1](LICENSE).
This file covers the third-party crates MediaGit depends on, which are licensed
separately by their respective copyright holders and remain under their own terms.

Two obligations make this file mandatory rather than courteous:

- **MPL-2.0 (file-level copyleft).** The `symphonia*` audio family and `mp4parse`
  ship inside the `mediagit` binary via `mediagit-media`. Those files remain under
  MPL-2.0 and their source must remain available. MPL-2.0 does not extend to
  MediaGit's own code.
- **Apache-2.0 / BSD / MIT attribution.** These require copyright and permission
  notices to be preserved in redistributions.

**648 third-party packages** across **36 license expressions**.

> Generated from `cargo metadata --all-features`. Deliberately over-inclusive:
> every resolved package is listed, including dev- and build-only dependencies.
> Attributing a crate that is not shipped is harmless; omitting one that is
> shipped is the compliance failure this file exists to prevent.

## Summary

| License | Packages |
|---|--:|
| `MIT OR Apache-2.0` | 294 |
| `MIT` | 99 |
| `Apache-2.0 OR MIT` | 70 |
| `Apache-2.0` | 55 |
| `MIT/Apache-2.0` | 29 |
| `Unicode-3.0` | 18 |
| `MPL-2.0` | 17 |
| `Unlicense OR MIT` | 12 |
| `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 7 |
| `BSD-3-Clause` | 5 |
| `ISC` | 4 |
| `Apache-2.0 OR ISC OR MIT` | 3 |
| `BSD-2-Clause` | 3 |
| `MIT OR Apache-2.0 OR Zlib` | 3 |
| `Apache-2.0/MIT` | 2 |
| `BSD-2-Clause OR Apache-2.0 OR MIT` | 2 |
| `BSD-3-Clause OR Apache-2.0` | 2 |
| `CC0-1.0 OR MIT-0 OR Apache-2.0` | 2 |
| `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 2 |
| `Unlicense/MIT` | 2 |
| `Zlib OR Apache-2.0 OR MIT` | 2 |
| `(Apache-2.0 OR MIT) AND BSD-3-Clause` | 1 |
| `(MIT OR Apache-2.0) AND Apache-2.0` | 1 |
| `(MIT OR Apache-2.0) AND Unicode-3.0` | 1 |
| `0BSD OR MIT OR Apache-2.0` | 1 |
| `Apache-2.0 / MIT` | 1 |
| `Apache-2.0 AND ISC` | 1 |
| `Apache-2.0 OR BSL-1.0` | 1 |
| `BSD-3-Clause AND MIT` | 1 |
| `BSD-3-Clause/MIT` | 1 |
| `CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception` | 1 |
| `CDLA-Permissive-2.0` | 1 |
| `MIT AND BSD-3-Clause` | 1 |
| `MIT OR Apache-2.0 OR BSD-1-Clause` | 1 |
| `MIT OR Zlib OR Apache-2.0` | 1 |
| `Zlib` | 1 |

## Packages by license

### MIT OR Apache-2.0

- **aead** 0.5.2 — https://github.com/RustCrypto/traits
- **aes** 0.8.4 — https://github.com/RustCrypto/block-ciphers
- **ahash** 0.8.12 — https://github.com/tkaitchuck/ahash
- **allocator-api2** 0.2.21 — https://github.com/zakarumych/allocator-api2
- **anes** 0.1.6 — https://github.com/zrzka/anes-rs
- **anstream** 1.0.0 — https://github.com/rust-cli/anstyle.git
- **anstyle** 1.0.14 — https://github.com/rust-cli/anstyle.git
- **anstyle-parse** 1.0.0 — https://github.com/rust-cli/anstyle.git
- **anstyle-query** 1.1.5 — https://github.com/rust-cli/anstyle.git
- **anstyle-wincon** 3.0.11 — https://github.com/rust-cli/anstyle.git
- **anyhow** 1.0.104 — https://github.com/dtolnay/anyhow
- **apple-native-keyring-store** 1.0.1 — https://github.com/open-source-cooperative/apple-native-keyring-store.git
- **arc-swap** 1.9.2 — https://github.com/vorner/arc-swap
- **argon2** 0.5.3 — https://github.com/RustCrypto/password-hashes/tree/master/argon2
- **arrayvec** 0.7.8 — https://github.com/bluss/arrayvec
- **asn1-rs** 0.7.2 — https://github.com/rusticata/asn1-rs.git
- **asn1-rs-derive** 0.6.0 — https://github.com/rusticata/asn1-rs.git
- **assert_cmd** 2.2.2 — https://github.com/assert-rs/assert_cmd.git
- **async-broadcast** 0.7.2 — https://github.com/smol-rs/async-broadcast
- **async-recursion** 1.1.1 — https://github.com/dcchut/async-recursion
- **async-trait** 0.1.91 — https://github.com/dtolnay/async-trait
- **atomic-polyfill** 1.0.3 — https://github.com/embassy-rs/atomic-polyfill
- **base64** 0.22.1 — https://github.com/marshallpierce/rust-base64
- **bitflags** 2.13.1 — https://github.com/bitflags/bitflags
- **bitreader** 0.3.11 — https://github.com/irauta/bitreader
- **blake2** 0.10.6 — https://github.com/RustCrypto/hashes
- **block-buffer** 0.10.4 — https://github.com/RustCrypto/utils
- **block-buffer** 0.12.1 — https://github.com/RustCrypto/utils
- **block-padding** 0.3.3 — https://github.com/RustCrypto/utils
- **blowfish** 0.10.0 — https://github.com/RustCrypto/block-ciphers
- **bstr** 1.13.0 — https://github.com/BurntSushi/bstr
- **bumpalo** 3.20.3 — https://github.com/fitzgen/bumpalo
- **cast** 0.3.0 — https://github.com/japaric/cast.rs
- **cbc** 0.1.2 — https://github.com/RustCrypto/block-modes
- **cc** 1.3.0 — https://github.com/rust-lang/cc-rs
- **cfg-if** 1.0.4 — https://github.com/rust-lang/cfg-if
- **chacha20** 0.10.1 — https://github.com/RustCrypto/stream-ciphers
- **chrono** 0.4.45 — https://github.com/chronotope/chrono
- **cipher** 0.4.4 — https://github.com/RustCrypto/traits
- **cipher** 0.5.2 — https://github.com/RustCrypto/traits
- **clap** 4.6.4 — https://github.com/clap-rs/clap
- **clap_builder** 4.6.2 — https://github.com/clap-rs/clap
- **clap_complete** 4.6.7 — https://github.com/clap-rs/clap
- **clap_derive** 4.6.4 — https://github.com/clap-rs/clap
- **clap_lex** 1.1.0 — https://github.com/clap-rs/clap
- **cobs** 0.3.0 — https://github.com/jamesmunns/cobs.rs
- **colorchoice** 1.0.5 — https://github.com/rust-cli/anstyle.git
- **core-foundation** 0.10.1 — https://github.com/servo/core-foundation-rs
- **core-foundation-sys** 0.8.7 — https://github.com/servo/core-foundation-rs
- **cpufeatures** 0.2.17 — https://github.com/RustCrypto/utils
- **cpufeatures** 0.3.0 — https://github.com/RustCrypto/utils
- **crc-fast** 1.10.0 — https://github.com/awesomized/crc-fast-rust
- **crc32fast** 1.5.0 — https://github.com/srijs/rust-crc32fast
- **critical-section** 1.2.0 — https://github.com/rust-embedded/critical-section
- **crossbeam-channel** 0.5.16 — https://github.com/crossbeam-rs/crossbeam
- **crossbeam-deque** 0.8.7 — https://github.com/crossbeam-rs/crossbeam
- **crossbeam-epoch** 0.9.20 — https://github.com/crossbeam-rs/crossbeam
- **crossbeam-utils** 0.8.22 — https://github.com/crossbeam-rs/crossbeam
- **crypto-common** 0.1.7 — https://github.com/RustCrypto/traits
- **crypto-common** 0.2.2 — https://github.com/RustCrypto/traits
- **ctr** 0.9.2 — https://github.com/RustCrypto/block-modes
- **defmt** 1.1.1 — https://github.com/knurling-rs/defmt
- **defmt-macros** 1.1.1 — https://github.com/knurling-rs/defmt
- **defmt-parser** 1.0.0 — https://github.com/knurling-rs/defmt
- **der-parser** 10.0.0 — https://github.com/rusticata/der-parser.git
- **deranged** 0.5.8 — https://github.com/jhpratt/deranged
- **digest** 0.10.7 — https://github.com/RustCrypto/traits
- **digest** 0.11.3 — https://github.com/RustCrypto/traits
- **displaydoc** 0.2.6 — https://github.com/yaahc/displaydoc
- **dyn-clone** 1.0.20 — https://github.com/dtolnay/dyn-clone
- **either** 1.16.0 — https://github.com/rayon-rs/either
- **embedded-io** 0.4.0 — https://github.com/embassy-rs/embedded-io
- **embedded-io** 0.6.1 — https://github.com/rust-embedded/embedded-hal
- **enumflags2** 0.7.12 — https://github.com/meithecatte/enumflags2
- **enumflags2_derive** 0.7.12 — https://github.com/meithecatte/enumflags2
- **errno** 0.3.14 — https://github.com/lambda-fairy/rust-errno
- **fdeflate** 0.3.7 — https://github.com/image-rs/fdeflate
- **find-msvc-tools** 0.1.9 — https://github.com/rust-lang/cc-rs
- **flate2** 1.1.9 — https://github.com/rust-lang/flate2-rs
- **form_urlencoded** 1.2.2 — https://github.com/servo/rust-url
- **fs-err** 3.3.1 — https://github.com/andrewhickman/fs-err
- **futures** 0.3.33 — https://github.com/rust-lang/futures-rs
- **futures-channel** 0.3.33 — https://github.com/rust-lang/futures-rs
- **futures-core** 0.3.33 — https://github.com/rust-lang/futures-rs
- **futures-executor** 0.3.33 — https://github.com/rust-lang/futures-rs
- **futures-io** 0.3.33 — https://github.com/rust-lang/futures-rs
- **futures-macro** 0.3.33 — https://github.com/rust-lang/futures-rs
- **futures-sink** 0.3.33 — https://github.com/rust-lang/futures-rs
- **futures-task** 0.3.33 — https://github.com/rust-lang/futures-rs
- **futures-util** 0.3.33 — https://github.com/rust-lang/futures-rs
- **getrandom** 0.2.17 — https://github.com/rust-random/getrandom
- **getrandom** 0.3.4 — https://github.com/rust-random/getrandom
- **getrandom** 0.4.3 — https://github.com/rust-random/getrandom
- **glob** 0.3.4 — https://github.com/rust-lang/glob
- **gloo-timers** 0.3.0 — https://github.com/rustwasm/gloo/tree/master/crates/timers
- **half** 2.7.1 — https://github.com/VoidStarKat/half-rs
- **hash32** 0.2.1 — https://github.com/japaric/hash32
- **hashbrown** 0.12.3 — https://github.com/rust-lang/hashbrown
- **hashbrown** 0.13.2 — https://github.com/rust-lang/hashbrown
- **hashbrown** 0.14.5 — https://github.com/rust-lang/hashbrown
- **hashbrown** 0.16.1 — https://github.com/rust-lang/hashbrown
- **hashbrown** 0.17.1 — https://github.com/rust-lang/hashbrown
- **heapless** 0.7.17 — https://github.com/japaric/heapless
- **heck** 0.5.0 — https://github.com/withoutboats/heck
- **hermit-abi** 0.5.2 — https://github.com/hermit-os/hermit-rs
- **hex** 0.4.3 — https://github.com/KokaKiwi/rust-hex
- **hkdf** 0.12.4 — https://github.com/RustCrypto/KDFs/
- **hmac** 0.12.1 — https://github.com/RustCrypto/MACs
- **hmac** 0.13.0 — https://github.com/RustCrypto/MACs
- **http** 0.2.12 — https://github.com/hyperium/http
- **http** 1.4.2 — https://github.com/hyperium/http
- **httparse** 1.10.1 — https://github.com/seanmonstar/httparse
- **httpdate** 1.0.3 — https://github.com/pyfisch/httpdate
- **husky-rs** 0.3.3 — https://github.com/pplmx/husky-rs
- **hybrid-array** 0.4.13 — https://github.com/RustCrypto/hybrid-array
- **hyper-timeout** 0.5.2 — https://github.com/hjr3/hyper-timeout
- **iana-time-zone** 0.1.65 — https://github.com/strawlab/iana-time-zone
- **iana-time-zone-haiku** 0.1.2 — https://github.com/strawlab/iana-time-zone
- **idna** 1.1.0 — https://github.com/servo/rust-url/
- **image** 0.25.10 — https://github.com/image-rs/image
- **image-webp** 0.2.4 — https://github.com/image-rs/image-webp
- **image_hasher** 3.1.1 — http://github.com/qarmin/img_hash
- **inout** 0.1.4 — https://github.com/RustCrypto/utils
- **inout** 0.2.2 — https://github.com/RustCrypto/utils
- **ipnet** 2.12.0 — https://github.com/krisprice/ipnet
- **is_terminal_polyfill** 1.70.2 — https://github.com/polyfill-rs/is_terminal_polyfill
- **itertools** 0.13.0 — https://github.com/rust-itertools/itertools
- **itertools** 0.14.0 — https://github.com/rust-itertools/itertools
- **itoa** 1.0.18 — https://github.com/dtolnay/itoa
- **jni** 0.22.4 — https://github.com/jni-rs/jni-rs
- **jni-macros** 0.22.4 — https://github.com/jni-rs/jni-rs
- **jni-sys** 0.4.1 — https://github.com/jni-rs/jni-sys
- **jni-sys-macros** 0.4.1 — https://github.com/jni-rs/jni-sys
- **jobserver** 0.1.35 — https://github.com/rust-lang/jobserver-rs
- **js-sys** 0.3.103 — https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys
- **keyring** 4.1.5 — https://github.com/open-source-cooperative/keyring-rs
- **keyring-core** 1.0.0 — https://github.com/open-source-cooperative/keyring-core.git
- **lazy_static** 1.5.0 — https://github.com/rust-lang-nursery/lazy-static.rs
- **libc** 0.2.189 — https://github.com/rust-lang/libc
- **lock_api** 0.4.14 — https://github.com/Amanieu/parking_lot
- **log** 0.4.33 — https://github.com/rust-lang/log
- **md-5** 0.11.0 — https://github.com/RustCrypto/hashes
- **memmap2** 0.9.11 — https://github.com/RazrFalcon/memmap2-rs
- **mime** 0.3.17 — https://github.com/hyperium/mime
- **num** 0.4.3 — https://github.com/rust-num/num
- **num-bigint** 0.4.8 — https://github.com/rust-num/num-bigint
- **num-complex** 0.4.6 — https://github.com/rust-num/num-complex
- **num-conv** 0.2.2 — https://github.com/jhpratt/num-conv
- **num-integer** 0.1.46 — https://github.com/rust-num/num-integer
- **num-iter** 0.1.46 — https://github.com/rust-num/num-iter
- **num-rational** 0.4.2 — https://github.com/rust-num/num-rational
- **num-traits** 0.2.19 — https://github.com/rust-num/num-traits
- **num_cpus** 1.17.0 — https://github.com/seanmonstar/num_cpus
- **oid-registry** 0.8.1 — https://github.com/rusticata/oid-registry.git
- **once_cell** 1.21.4 — https://github.com/matklad/once_cell
- **once_cell_polyfill** 1.70.2 — https://github.com/polyfill-rs/once_cell_polyfill
- **opaque-debug** 0.3.1 — https://github.com/RustCrypto/utils
- **openssl-probe** 0.2.1 — https://github.com/rustls/openssl-probe
- **ordered-stream** 0.2.0 — https://github.com/danieldg/ordered-stream
- **parking_lot** 0.12.5 — https://github.com/Amanieu/parking_lot
- **parking_lot_core** 0.9.12 — https://github.com/Amanieu/parking_lot
- **password-hash** 0.5.0 — https://github.com/RustCrypto/traits/tree/master/password-hash
- **pbkdf2** 0.12.2 — https://github.com/RustCrypto/password-hashes/tree/master/pbkdf2
- **percent-encoding** 2.3.2 — https://github.com/servo/rust-url/
- **pin-utils** 0.1.0 — https://github.com/rust-lang-nursery/pin-utils
- **piper** 0.2.5 — https://github.com/smol-rs/piper
- **pkg-config** 0.3.33 — https://github.com/rust-lang/pkg-config-rs
- **png** 0.18.1 — https://github.com/image-rs/image-png
- **postcard** 1.1.3 — https://github.com/jamesmunns/postcard
- **powerfmt** 0.2.0 — https://github.com/jhpratt/powerfmt
- **ppv-lite86** 0.2.21 — https://github.com/cryptocorrosion/cryptocorrosion
- **predicates** 3.1.4 — https://github.com/assert-rs/predicates-rs
- **predicates-core** 1.0.10 — https://github.com/assert-rs/predicates-rs
- **predicates-tree** 1.0.13 — https://github.com/assert-rs/predicates-rs
- **primal-check** 0.3.4 — https://github.com/huonw/primal
- **proc-macro-crate** 3.5.0 — https://github.com/bkchr/proc-macro-crate
- **proc-macro2** 1.0.107 — https://github.com/dtolnay/proc-macro2
- **procfs** 0.17.0 — https://github.com/eminence/procfs
- **procfs-core** 0.17.0 — https://github.com/eminence/procfs
- **proptest** 1.11.0 — https://github.com/proptest-rs/proptest
- **quote** 1.0.47 — https://github.com/dtolnay/quote
- **rand** 0.10.2 — https://github.com/rust-random/rand
- **rand** 0.8.7 — https://github.com/rust-random/rand
- **rand** 0.9.5 — https://github.com/rust-random/rand
- **rand_chacha** 0.3.1 — https://github.com/rust-random/rand
- **rand_chacha** 0.9.0 — https://github.com/rust-random/rand
- **rand_core** 0.10.1 — https://github.com/rust-random/rand_core
- **rand_core** 0.6.4 — https://github.com/rust-random/rand
- **rand_core** 0.9.5 — https://github.com/rust-random/rand
- **rand_xorshift** 0.4.0 — https://github.com/rust-random/rngs
- **rayon** 1.12.0 — https://github.com/rayon-rs/rayon
- **rayon-core** 1.13.0 — https://github.com/rayon-rs/rayon
- **rcgen** 0.14.8 — https://github.com/rustls/rcgen
- **ref-cast** 1.0.26 — https://github.com/dtolnay/ref-cast
- **ref-cast-impl** 1.0.26 — https://github.com/dtolnay/ref-cast
- **regex** 1.13.1 — https://github.com/rust-lang/regex
- **regex-automata** 0.4.16 — https://github.com/rust-lang/regex
- **regex-lite** 0.1.9 — https://github.com/rust-lang/regex
- **regex-syntax** 0.8.11 — https://github.com/rust-lang/regex
- **reqwest** 0.13.4 — https://github.com/seanmonstar/reqwest
- **roaring** 0.11.4 — https://github.com/RoaringBitmap/roaring-rs
- **rsa** 0.9.10 — https://github.com/RustCrypto/RSA
- **rustc_version** 0.4.1 — https://github.com/djc/rustc-version-rs
- **rustdct** 0.7.1 — https://github.com/ejmahler/rust_dct
- **rustfft** 6.4.1 — https://github.com/ejmahler/RustFFT
- **rustls-pki-types** 1.15.0 — https://github.com/rustls/pki-types
- **rustls-platform-verifier** 0.7.0 — https://github.com/rustls/rustls-platform-verifier
- **rustls-platform-verifier-android** 0.1.1 — https://github.com/rustls/rustls-platform-verifier
- **rustversion** 1.0.23 — https://github.com/dtolnay/rustversion
- **salsa20** 0.10.2 — https://github.com/RustCrypto/stream-ciphers
- **scopeguard** 1.2.0 — https://github.com/bluss/scopeguard
- **scrypt** 0.11.0 — https://github.com/RustCrypto/password-hashes/tree/master/scrypt
- **secret-service** 5.1.0 — https://github.com/hwchen/secret-service-rs.git
- **security-framework** 3.7.0 — https://github.com/kornelski/rust-security-framework
- **security-framework-sys** 2.17.0 — https://github.com/kornelski/rust-security-framework
- **semver** 1.0.28 — https://github.com/dtolnay/semver
- **serde** 1.0.229 — https://github.com/serde-rs/serde
- **serde_core** 1.0.229 — https://github.com/serde-rs/serde
- **serde_derive** 1.0.229 — https://github.com/serde-rs/serde
- **serde_json** 1.0.151 — https://github.com/serde-rs/json
- **serde_path_to_error** 0.1.20 — https://github.com/dtolnay/path-to-error
- **serde_repr** 0.1.21 — https://github.com/dtolnay/serde-repr
- **serde_spanned** 1.1.1 — https://github.com/toml-rs/toml
- **serde_with** 3.21.0 — https://github.com/jonasbb/serde_with/
- **serde_with_macros** 3.21.0 — https://github.com/jonasbb/serde_with/
- **serde_yaml** 0.9.34+deprecated — https://github.com/dtolnay/serde-yaml
- **sha1** 0.11.0 — https://github.com/RustCrypto/hashes
- **sha2** 0.10.9 — https://github.com/RustCrypto/hashes
- **sha2** 0.11.0 — https://github.com/RustCrypto/hashes
- **shlex** 2.0.1 — https://github.com/comex/rust-shlex
- **signal-hook-registry** 1.4.8 — https://github.com/vorner/signal-hook
- **simdutf8** 0.1.5 — https://github.com/rusticstuff/simdutf8
- **smallvec** 1.15.2 — https://github.com/servo/rust-smallvec
- **socket2** 0.6.5 — https://github.com/rust-lang/socket2
- **stable_deref_trait** 1.2.1 — https://github.com/storyyeller/stable_deref_trait
- **static_assertions** 1.1.0 — https://github.com/nvzqz/static-assertions-rs
- **strength_reduce** 0.2.4 — http://github.com/ejmahler/strength_reduce
- **syn** 2.0.119 — https://github.com/dtolnay/syn
- **syn** 3.0.3 — https://github.com/dtolnay/syn
- **tempfile** 3.27.0 — https://github.com/Stebalien/tempfile
- **thiserror** 1.0.69 — https://github.com/dtolnay/thiserror
- **thiserror** 2.0.19 — https://github.com/dtolnay/thiserror
- **thiserror-impl** 1.0.69 — https://github.com/dtolnay/thiserror
- **thiserror-impl** 2.0.19 — https://github.com/dtolnay/thiserror
- **thread_local** 1.1.10 — https://github.com/Amanieu/thread_local-rs
- **time** 0.3.54 — https://github.com/time-rs/time
- **time-core** 0.1.9 — https://github.com/time-rs/time
- **time-macros** 0.2.32 — https://github.com/time-rs/time
- **tokio-rustls** 0.26.4 — https://github.com/rustls/tokio-rustls
- **toml** 1.1.3+spec-1.1.0 — https://github.com/toml-rs/toml
- **toml_datetime** 1.1.1+spec-1.1.0 — https://github.com/toml-rs/toml
- **toml_edit** 0.25.13+spec-1.1.0 — https://github.com/toml-rs/toml
- **toml_parser** 1.1.2+spec-1.1.0 — https://github.com/toml-rs/toml
- **toml_writer** 1.1.2+spec-1.1.0 — https://github.com/toml-rs/toml
- **tower_governor** 0.8.0 — https://github.com/benwis/tower-governor
- **transpose** 0.2.3 — https://github.com/ejmahler/transpose
- **typenum** 1.20.1 — https://github.com/paholg/typenum
- **unarray** 0.1.4 — https://github.com/cameron1024/unarray
- **unicase** 2.9.0 — https://github.com/seanmonstar/unicase
- **unicode-width** 0.2.2 — https://github.com/unicode-rs/unicode-width
- **universal-hash** 0.5.1 — https://github.com/RustCrypto/traits
- **url** 2.5.8 — https://github.com/servo/rust-url
- **wasm-bindgen** 0.2.126 — https://github.com/wasm-bindgen/wasm-bindgen
- **wasm-bindgen-futures** 0.4.76 — https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/futures
- **wasm-bindgen-macro** 0.2.126 — https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro
- **wasm-bindgen-macro-support** 0.2.126 — https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro-support
- **wasm-bindgen-shared** 0.2.126 — https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared
- **wasm-streams** 0.5.0 — https://github.com/MattiasBuelens/wasm-streams/
- **web-sys** 0.3.103 — https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/web-sys
- **web-time** 1.1.0 — https://github.com/daxpedda/web-time
- **weezl** 0.1.12 — https://github.com/image-rs/weezl
- **windows-core** 0.62.2 — https://github.com/microsoft/windows-rs
- **windows-implement** 0.60.2 — https://github.com/microsoft/windows-rs
- **windows-interface** 0.59.3 — https://github.com/microsoft/windows-rs
- **windows-link** 0.2.1 — https://github.com/microsoft/windows-rs
- **windows-native-keyring-store** 1.1.0 — https://github.com/open-source-cooperative/windows-native-keyring-store.git
- **windows-result** 0.4.1 — https://github.com/microsoft/windows-rs
- **windows-strings** 0.5.1 — https://github.com/microsoft/windows-rs
- **windows-sys** 0.52.0 — https://github.com/microsoft/windows-rs
- **windows-sys** 0.59.0 — https://github.com/microsoft/windows-rs
- **windows-sys** 0.61.2 — https://github.com/microsoft/windows-rs
- **windows-targets** 0.52.6 — https://github.com/microsoft/windows-rs
- **windows_aarch64_gnullvm** 0.52.6 — https://github.com/microsoft/windows-rs
- **windows_aarch64_msvc** 0.52.6 — https://github.com/microsoft/windows-rs
- **windows_i686_gnu** 0.52.6 — https://github.com/microsoft/windows-rs
- **windows_i686_gnullvm** 0.52.6 — https://github.com/microsoft/windows-rs
- **windows_i686_msvc** 0.52.6 — https://github.com/microsoft/windows-rs
- **windows_x86_64_gnu** 0.52.6 — https://github.com/microsoft/windows-rs
- **windows_x86_64_gnullvm** 0.52.6 — https://github.com/microsoft/windows-rs
- **windows_x86_64_msvc** 0.52.6 — https://github.com/microsoft/windows-rs
- **x509-parser** 0.18.1 — https://github.com/rusticata/x509-parser.git
- **yasna** 0.6.0 — https://github.com/qnighy/yasna.rs
- **zbus-secret-service-keyring-store** 1.0.0 — https://github.com/open-source-cooperative/zbus-secret-service-keyring-store.git
- **zstd-safe** 7.2.4 — https://github.com/gyscos/zstd-rs

### MIT

- **alloca** 0.4.0 — https://github.com/playXE/alloca-rs
- **axum** 0.8.9 — https://github.com/tokio-rs/axum
- **axum-core** 0.5.6 — https://github.com/tokio-rs/axum
- **axum-server** 0.8.0 — https://github.com/programatik29/axum-server
- **base64-simd** 0.8.0 — https://github.com/Nugine/simd
- **bcrypt** 0.19.2 — https://github.com/Keats/rust-bcrypt
- **bytes** 1.12.1 — https://github.com/tokio-rs/bytes
- **combine** 4.6.7 — https://github.com/Marwes/combine
- **console** 0.16.4 — https://github.com/console-rs/console
- **crunchy** 0.2.4 — https://github.com/eira-fransham/crunchy
- **darling** 0.23.0 — https://github.com/TedDriggs/darling
- **darling_core** 0.23.0 — https://github.com/TedDriggs/darling
- **darling_macro** 0.23.0 — https://github.com/TedDriggs/darling
- **dashmap** 6.2.1 — https://github.com/xacrimon/dashmap
- **data-encoding** 2.11.0 — https://github.com/ia0/data-encoding
- **dialoguer** 0.12.0 — https://github.com/console-rs/dialoguer
- **difflib** 0.4.0 — https://github.com/DimaKudosh/difflib
- **endi** 1.1.1 — https://github.com/zeenix/endi
- **extended** 0.1.0 — https://github.com/depp/extended-rs
- **fastcdc** 4.0.1 — https://github.com/nlfiedler/fastcdc-rs
- **fax** 0.2.7 — https://github.com/pdf-rs/fax
- **float-cmp** 0.10.0 — https://github.com/mikedilger/float-cmp
- **generic-array** 0.14.7 — https://github.com/fizyk20/generic-array.git
- **governor** 0.10.4 — https://github.com/boinkor-net/governor.git
- **h2** 0.4.19 — https://github.com/hyperium/h2
- **http-body** 0.4.6 — https://github.com/hyperium/http-body
- **http-body** 1.1.0 — https://github.com/hyperium/http-body
- **http-body-util** 0.1.4 — https://github.com/hyperium/http-body
- **hyper** 1.11.0 — https://github.com/hyperium/hyper
- **hyper-util** 0.1.20 — https://github.com/hyperium/hyper-util
- **indicatif** 0.18.6 — https://github.com/console-rs/indicatif
- **jsonwebtoken** 10.4.0 — https://github.com/Keats/jsonwebtoken
- **libm** 0.2.16 — https://github.com/rust-lang/compiler-builtins
- **lru** 0.16.4 — https://github.com/jeromefroe/lru-rs.git
- **matchers** 0.2.0 — https://github.com/hawkw/matchers
- **memoffset** 0.9.1 — https://github.com/Gilnaa/memoffset
- **mime_guess** 2.0.5 — https://github.com/abonander/mime_guess
- **mio** 1.2.2 — https://github.com/tokio-rs/mio
- **nom** 7.1.3 — https://github.com/Geal/nom
- **nonempty** 0.7.0 — https://github.com/cloudhead/nonempty
- **nu-ansi-term** 0.50.3 — https://github.com/nushell/nu-ansi-term
- **oorandom** 11.1.5 — https://hg.sr.ht/~icefox/oorandom
- **outref** 0.5.2 — https://github.com/Nugine/outref
- **pem** 3.0.6 — https://github.com/jcreekmore/pem-rs.git
- **plotters** 0.3.7 — https://github.com/plotters-rs/plotters
- **plotters-backend** 0.3.7 — https://github.com/plotters-rs/plotters
- **plotters-svg** 0.3.7 — https://github.com/plotters-rs/plotters.git
- **protobuf** 3.7.2 — https://github.com/stepancheg/rust-protobuf/
- **protobuf-support** 3.7.2 — https://github.com/stepancheg/rust-protobuf/
- **quanta** 0.12.6 — https://github.com/metrics-rs/quanta
- **quick-xml** 0.39.4 — https://github.com/tafia/quick-xml
- **raw-cpuid** 11.6.0 — https://github.com/gz/rust-cpuid
- **redox_syscall** 0.5.18 — https://gitlab.redox-os.org/redox-os/syscall
- **schannel** 0.1.29 — https://github.com/steffengy/schannel-rs
- **schemars** 0.9.0 — https://github.com/GREsau/schemars
- **schemars** 1.2.1 — https://github.com/GREsau/schemars
- **sharded-slab** 0.1.7 — https://github.com/hawkw/sharded-slab
- **simd-adler32** 0.3.10 — https://github.com/mcountryman/simd-adler32
- **slab** 0.4.12 — https://github.com/tokio-rs/slab
- **spin** 0.10.1 — https://github.com/mvdnes/spin-rs.git
- **spin** 0.9.9 — https://github.com/mvdnes/spin-rs.git
- **strsim** 0.11.1 — https://github.com/rapidfuzz/strsim-rs
- **synstructure** 0.13.2 — https://github.com/mystor/synstructure
- **termtree** 0.5.1 — https://github.com/rust-cli/termtree
- **tiff** 0.11.3 — https://github.com/image-rs/image-tiff
- **tokio** 1.53.1 — https://github.com/tokio-rs/tokio
- **tokio-macros** 2.7.1 — https://github.com/tokio-rs/tokio
- **tokio-stream** 0.1.18 — https://github.com/tokio-rs/tokio
- **tokio-util** 0.7.19 — https://github.com/tokio-rs/tokio
- **tonic** 0.14.6 — https://github.com/hyperium/tonic
- **tonic-prost** 0.14.6 — https://github.com/hyperium/tonic
- **tower** 0.5.3 — https://github.com/tower-rs/tower
- **tower-http** 0.6.11 — https://github.com/tower-rs/tower-http
- **tower-layer** 0.3.3 — https://github.com/tower-rs/tower
- **tower-service** 0.3.3 — https://github.com/tower-rs/tower
- **tracing** 0.1.44 — https://github.com/tokio-rs/tracing
- **tracing-attributes** 0.1.31 — https://github.com/tokio-rs/tracing
- **tracing-core** 0.1.36 — https://github.com/tokio-rs/tracing
- **tracing-log** 0.2.0 — https://github.com/tokio-rs/tracing
- **tracing-opentelemetry** 0.33.0 — https://github.com/tokio-rs/tracing-opentelemetry
- **tracing-serde** 0.2.0 — https://github.com/tokio-rs/tracing
- **tracing-subscriber** 0.3.23 — https://github.com/tokio-rs/tracing
- **try-lock** 0.2.5 — https://github.com/seanmonstar/try-lock
- **uds_windows** 1.2.1 — https://github.com/haraldh/rust_uds_windows
- **unit-prefix** 0.5.2 — https://codeberg.org/commons-rs/unit-prefix
- **unsafe-libyaml** 0.2.11 — https://github.com/dtolnay/unsafe-libyaml
- **urlencoding** 2.1.3 — https://github.com/kornelski/rust_urlencoding
- **valuable** 0.1.1 — https://github.com/tokio-rs/valuable
- **vsimd** 0.8.0 — https://github.com/Nugine/simd
- **want** 0.3.1 — https://github.com/seanmonstar/want
- **winnow** 1.0.4 — https://github.com/winnow-rs/winnow
- **zbus** 5.18.0 — https://github.com/z-galaxy/zbus/
- **zbus_macros** 5.18.0 — https://github.com/z-galaxy/zbus/
- **zbus_names** 4.3.4 — https://github.com/z-galaxy/zbus/
- **zmij** 1.0.23 — https://github.com/dtolnay/zmij
- **zstd** 0.13.3 — https://github.com/gyscos/zstd-rs
- **zvariant** 5.13.1 — https://github.com/z-galaxy/zbus/
- **zvariant_derive** 5.13.1 — https://github.com/z-galaxy/zbus/
- **zvariant_utils** 3.5.0 — https://github.com/z-galaxy/zbus/

### Apache-2.0 OR MIT

- **aes-gcm** 0.10.3 — https://github.com/RustCrypto/AEADs
- **async-channel** 2.5.0 — https://github.com/smol-rs/async-channel
- **async-executor** 1.14.0 — https://github.com/smol-rs/async-executor
- **async-io** 2.6.0 — https://github.com/smol-rs/async-io
- **async-lock** 3.4.2 — https://github.com/smol-rs/async-lock
- **async-process** 2.5.0 — https://github.com/smol-rs/async-process
- **async-signal** 0.2.14 — https://github.com/smol-rs/async-signal
- **async-task** 4.7.1 — https://github.com/smol-rs/async-task
- **atomic-waker** 1.1.2 — https://github.com/smol-rs/atomic-waker
- **autocfg** 1.5.1 — https://github.com/cuviper/autocfg
- **base16ct** 0.2.0 — https://github.com/RustCrypto/formats/tree/master/base16ct
- **base64ct** 1.8.3 — https://github.com/RustCrypto/formats
- **bit-set** 0.8.0 — https://github.com/contain-rs/bit-set
- **bit-vec** 0.8.0 — https://github.com/contain-rs/bit-vec
- **bit-vec** 0.9.1 — https://github.com/contain-rs/bit-vec
- **blocking** 1.6.2 — https://github.com/smol-rs/blocking
- **cmov** 0.5.4 — https://github.com/RustCrypto/utils
- **concurrent-queue** 2.5.0 — https://github.com/smol-rs/concurrent-queue
- **const-oid** 0.10.2 — https://github.com/RustCrypto/formats
- **const-oid** 0.9.6 — https://github.com/RustCrypto/formats/tree/master/const-oid
- **criterion** 0.8.2 — https://github.com/criterion-rs/criterion.rs
- **criterion-plot** 0.8.2 — https://github.com/criterion-rs/criterion.rs
- **crypto-bigint** 0.5.5 — https://github.com/RustCrypto/crypto-bigint
- **ctutils** 0.4.2 — https://github.com/RustCrypto/utils
- **der** 0.7.10 — https://github.com/RustCrypto/formats/tree/master/der
- **ecdsa** 0.16.9 — https://github.com/RustCrypto/signatures/tree/master/ecdsa
- **ed25519** 2.2.3 — https://github.com/RustCrypto/signatures/tree/master/ed25519
- **elliptic-curve** 0.13.8 — https://github.com/RustCrypto/traits/tree/master/elliptic-curve
- **encode_unicode** 1.0.0 — https://github.com/tormol/encode_unicode
- **equivalent** 1.0.2 — https://github.com/indexmap-rs/equivalent
- **event-listener** 5.4.1 — https://github.com/smol-rs/event-listener
- **event-listener-strategy** 0.5.4 — https://github.com/smol-rs/event-listener-strategy
- **fastrand** 2.5.0 — https://github.com/smol-rs/fastrand
- **futures-lite** 2.6.1 — https://github.com/smol-rs/futures-lite
- **ghash** 0.5.1 — https://github.com/RustCrypto/universal-hashes
- **idna_adapter** 1.2.2 — https://github.com/hsivonen/idna_adapter
- **indexmap** 1.9.3 — https://github.com/bluss/indexmap
- **indexmap** 2.14.0 — https://github.com/indexmap-rs/indexmap
- **md5** 0.8.1 — https://github.com/stainless-steel/md5
- **p256** 0.13.2 — https://github.com/RustCrypto/elliptic-curves/tree/master/p256
- **p384** 0.13.1 — https://github.com/RustCrypto/elliptic-curves/tree/master/p384
- **p521** 0.13.3 — https://github.com/RustCrypto/elliptic-curves/tree/master/p521
- **parking** 2.2.1 — https://github.com/smol-rs/parking
- **pem-rfc7468** 0.7.0 — https://github.com/RustCrypto/formats/tree/master/pem-rfc7468
- **pin-project** 1.1.13 — https://github.com/taiki-e/pin-project
- **pin-project-internal** 1.1.13 — https://github.com/taiki-e/pin-project
- **pin-project-lite** 0.2.17 — https://github.com/taiki-e/pin-project-lite
- **pkcs1** 0.7.5 — https://github.com/RustCrypto/formats/tree/master/pkcs1
- **pkcs5** 0.7.1 — https://github.com/RustCrypto/formats/tree/master/pkcs5
- **pkcs8** 0.10.2 — https://github.com/RustCrypto/formats/tree/master/pkcs8
- **polling** 3.11.0 — https://github.com/smol-rs/polling
- **polyval** 0.6.2 — https://github.com/RustCrypto/universal-hashes
- **portable-atomic** 1.14.0 — https://github.com/taiki-e/portable-atomic
- **portable-atomic-util** 0.2.7 — https://github.com/taiki-e/portable-atomic-util
- **primeorder** 0.13.6 — https://github.com/RustCrypto/elliptic-curves/tree/master/primeorder
- **rfc6979** 0.4.0 — https://github.com/RustCrypto/signatures/tree/master/rfc6979
- **sec1** 0.7.3 — https://github.com/RustCrypto/formats/tree/master/sec1
- **secrecy** 0.10.3 — https://github.com/iqlusioninc/crates/tree/main/secrecy
- **signature** 2.2.0 — https://github.com/RustCrypto/traits/tree/master/signature
- **simd_cesu8** 1.2.0 — https://github.com/seancroach/simd_cesu8
- **spki** 0.7.3 — https://github.com/RustCrypto/formats/tree/master/spki
- **ssh-cipher** 0.2.0 — https://github.com/RustCrypto/SSH/tree/master/ssh-cipher
- **ssh-encoding** 0.2.0 — https://github.com/RustCrypto/SSH/tree/master/ssh-encoding
- **ssh-key** 0.6.7 — https://github.com/RustCrypto/SSH/tree/master/ssh-key
- **tinytemplate** 1.2.1 — https://github.com/bheisler/TinyTemplate
- **utf8_iter** 1.0.4 — https://github.com/hsivonen/utf8_iter
- **utf8parse** 0.2.2 — https://github.com/alacritty/vte
- **uuid** 1.24.0 — https://github.com/uuid-rs/uuid
- **zeroize** 1.9.0 — https://github.com/RustCrypto/utils
- **zeroize_derive** 1.5.0 — https://github.com/RustCrypto/utils

### Apache-2.0

- **aws-config** 1.10.0 — https://github.com/smithy-lang/smithy-rs
- **aws-credential-types** 1.3.0 — https://github.com/smithy-lang/smithy-rs
- **aws-runtime** 1.9.0 — https://github.com/smithy-lang/smithy-rs
- **aws-sdk-s3** 1.139.0 — https://github.com/awslabs/aws-sdk-rust
- **aws-sdk-sts** 1.109.0 — https://github.com/awslabs/aws-sdk-rust
- **aws-sigv4** 1.5.1 — https://github.com/smithy-lang/smithy-rs
- **aws-smithy-async** 1.3.0 — https://github.com/smithy-lang/smithy-rs
- **aws-smithy-checksums** 0.65.0 — https://github.com/smithy-lang/smithy-rs
- **aws-smithy-eventstream** 0.61.1 — https://github.com/smithy-lang/smithy-rs
- **aws-smithy-http** 0.64.0 — https://github.com/smithy-lang/smithy-rs
- **aws-smithy-http-client** 1.2.0 — https://github.com/smithy-lang/smithy-rs
- **aws-smithy-json** 0.63.0 — https://github.com/smithy-lang/smithy-rs
- **aws-smithy-observability** 0.3.0 — https://github.com/awslabs/smithy-rs
- **aws-smithy-query** 0.62.0 — https://github.com/smithy-lang/smithy-rs
- **aws-smithy-runtime** 1.12.0 — https://github.com/smithy-lang/smithy-rs
- **aws-smithy-runtime-api** 1.13.0 — https://github.com/smithy-lang/smithy-rs
- **aws-smithy-runtime-api-macros** 1.1.0 — https://github.com/smithy-lang/smithy-rs
- **aws-smithy-schema** 0.2.0 — https://github.com/smithy-lang/smithy-rs
- **aws-smithy-types** 1.6.1 — https://github.com/smithy-lang/smithy-rs
- **aws-smithy-xml** 0.62.0 — https://github.com/smithy-lang/smithy-rs
- **aws-types** 1.5.0 — https://github.com/smithy-lang/smithy-rs
- **backon** 1.6.0 — https://github.com/Xuanwo/backon
- **ciborium** 0.2.2 — https://github.com/enarx/ciborium
- **ciborium-io** 0.2.2 — https://github.com/enarx/ciborium
- **ciborium-ll** 0.2.2 — https://github.com/enarx/ciborium
- **google-cloud-auth** 1.14.0 — https://github.com/googleapis/google-cloud-rust/tree/main
- **google-cloud-gax** 1.12.0 — https://github.com/googleapis/google-cloud-rust/tree/main
- **google-cloud-gax-internal** 0.7.15 — https://github.com/googleapis/google-cloud-rust/tree/main
- **google-cloud-iam-v1** 1.11.0 — https://github.com/googleapis/google-cloud-rust/tree/main
- **google-cloud-longrunning** 1.12.0 — https://github.com/googleapis/google-cloud-rust/tree/main
- **google-cloud-lro** 1.9.0 — https://github.com/googleapis/google-cloud-rust/tree/main
- **google-cloud-rpc** 1.6.0 — https://github.com/googleapis/google-cloud-rust/tree/main
- **google-cloud-storage** 1.16.0 — https://github.com/googleapis/google-cloud-rust/tree/main
- **google-cloud-type** 1.6.0 — https://github.com/googleapis/google-cloud-rust/tree/main
- **google-cloud-wkt** 1.6.0 — https://github.com/googleapis/google-cloud-rust/tree/main
- **mea** 0.6.4 — https://github.com/fast/mea
- **nonzero_ext** 0.3.0 — https://github.com/antifuchs/nonzero_ext
- **normalize-line-endings** 0.3.0 — https://github.com/derekdreery/normalize-line-endings
- **opendal** 0.57.0 — https://github.com/apache/opendal
- **opendal-core** 0.57.0 — https://github.com/apache/opendal
- **opendal-layer-retry** 0.57.0 — https://github.com/apache/opendal
- **opendal-layer-timeout** 0.57.0 — https://github.com/apache/opendal
- **opendal-service-azblob** 0.57.0 — https://github.com/apache/opendal
- **opendal-service-azure-common** 0.57.0 — https://github.com/apache/opendal
- **opentelemetry** 0.32.0 — https://github.com/open-telemetry/opentelemetry-rust/tree/main/opentelemetry
- **opentelemetry-semantic-conventions** 0.32.1 — https://github.com/open-telemetry/opentelemetry-rust/tree/main/opentelemetry-semantic-conventions
- **opentelemetry_sdk** 0.32.1 — https://github.com/open-telemetry/opentelemetry-rust/tree/main/opentelemetry-sdk
- **prometheus** 0.14.0 — https://github.com/tikv/rust-prometheus
- **prost** 0.14.4 — https://github.com/tokio-rs/prost
- **prost-derive** 0.14.4 — https://github.com/tokio-rs/prost
- **prost-types** 0.14.4 — https://github.com/tokio-rs/prost
- **reqsign-azure-storage** 3.1.0 — https://github.com/apache/opendal-reqsign
- **reqsign-core** 3.1.0 — https://github.com/apache/opendal-reqsign
- **reqsign-file-read-tokio** 3.0.2 — https://github.com/apache/opendal-reqsign
- **sync_wrapper** 1.0.2 — https://github.com/Actyx/sync_wrapper

### MIT/Apache-2.0

- **android_system_properties** 0.1.5 — https://github.com/nical/android_system_properties
- **asn1-rs-impl** 0.2.0 — https://github.com/rusticata/asn1-rs.git
- **bitflags** 1.3.2 — https://github.com/bitflags/bitflags
- **bs58** 0.5.1 — https://github.com/Nullus157/bs58-rs
- **curve25519-dalek-derive** 0.1.1 — https://github.com/dalek-cryptography/curve25519-dalek
- **fallible_collections** 0.4.9 — https://github.com/vcombey/fallible_collections.git
- **ff** 0.13.1 — https://github.com/zkcrypto/ff
- **futures-timer** 3.0.4 — https://github.com/async-rs/futures-timer
- **group** 0.13.0 — https://github.com/zkcrypto/group
- **ident_case** 1.0.1 — https://github.com/TedDriggs/ident_case
- **minimal-lexical** 0.2.1 — https://github.com/Alexhuszagh/minimal-lexical
- **num-bigint-dig** 0.8.6 — https://github.com/dignifiedquire/num-bigint
- **page_size** 0.6.0 — https://github.com/Elzair/page_size_rs
- **psd** 0.3.5 — https://github.com/chinedufn/psd
- **quick-error** 1.2.3 — http://github.com/tailhook/quick-error
- **quick-error** 2.0.1 — http://github.com/tailhook/quick-error
- **rusticata-macros** 4.1.0 — https://github.com/rusticata/rusticata-macros.git
- **rusty-fork** 0.3.1 — https://github.com/altsysrq/rusty-fork
- **serde_urlencoded** 0.7.1 — https://github.com/nox/serde_urlencoded
- **shell-words** 1.1.1 — https://github.com/tmiasko/shell-words
- **spinning_top** 0.3.0 — https://github.com/rust-osdev/spinning_top
- **tagptr** 0.2.0 — https://github.com/oliver-giersch/tagptr.git
- **version_check** 0.9.5 — https://github.com/SergioBenitez/version_check
- **wait-timeout** 0.2.1 — https://github.com/alexcrichton/wait-timeout
- **winapi** 0.3.9 — https://github.com/retep998/winapi-rs
- **winapi-i686-pc-windows-gnu** 0.4.0 — https://github.com/retep998/winapi-rs
- **winapi-x86_64-pc-windows-gnu** 0.4.0 — https://github.com/retep998/winapi-rs
- **xmlparser** 0.13.6 — https://github.com/RazrFalcon/xmlparser
- **zstd-sys** 2.0.16+zstd.1.5.7 — https://github.com/gyscos/zstd-rs

### Unicode-3.0

- **icu_collections** 2.2.0 — https://github.com/unicode-org/icu4x
- **icu_locale_core** 2.2.0 — https://github.com/unicode-org/icu4x
- **icu_normalizer** 2.2.0 — https://github.com/unicode-org/icu4x
- **icu_normalizer_data** 2.2.0 — https://github.com/unicode-org/icu4x
- **icu_properties** 2.2.0 — https://github.com/unicode-org/icu4x
- **icu_properties_data** 2.2.0 — https://github.com/unicode-org/icu4x
- **icu_provider** 2.2.0 — https://github.com/unicode-org/icu4x
- **litemap** 0.8.2 — https://github.com/unicode-org/icu4x
- **potential_utf** 0.1.5 — https://github.com/unicode-org/icu4x
- **tinystr** 0.8.3 — https://github.com/unicode-org/icu4x
- **writeable** 0.6.3 — https://github.com/unicode-org/icu4x
- **yoke** 0.8.3 — https://github.com/unicode-org/icu4x
- **yoke-derive** 0.8.2 — https://github.com/unicode-org/icu4x
- **zerofrom** 0.1.8 — https://github.com/unicode-org/icu4x
- **zerofrom-derive** 0.1.7 — https://github.com/unicode-org/icu4x
- **zerotrie** 0.2.4 — https://github.com/unicode-org/icu4x
- **zerovec** 0.11.6 — https://github.com/unicode-org/icu4x
- **zerovec-derive** 0.11.3 — https://github.com/unicode-org/icu4x

### MPL-2.0

- **mp4parse** 0.17.0 — https://github.com/mozilla/mp4parse-rust
- **symphonia** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-bundle-flac** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-bundle-mp3** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-codec-aac** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-codec-adpcm** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-codec-alac** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-codec-pcm** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-codec-vorbis** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-core** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-format-caf** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-format-isomp4** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-format-mkv** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-format-ogg** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-format-riff** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-metadata** 0.5.5 — https://github.com/pdeljanov/Symphonia
- **symphonia-utils-xiph** 0.5.5 — https://github.com/pdeljanov/Symphonia

### Unlicense OR MIT

- **aho-corasick** 1.1.4 — https://github.com/BurntSushi/aho-corasick
- **byteorder** 1.5.0 — https://github.com/BurntSushi/byteorder
- **byteorder-lite** 0.1.0 — https://github.com/image-rs/byteorder-lite
- **globset** 0.4.19 — https://github.com/BurntSushi/ripgrep/tree/master/crates/globset
- **ignore** 0.4.31 — https://github.com/BurntSushi/ripgrep/tree/master/crates/ignore
- **jiff** 0.2.34 — https://github.com/BurntSushi/jiff
- **jiff-core** 0.1.0 — https://github.com/BurntSushi/jiff
- **jiff-static** 0.2.34 — https://github.com/BurntSushi/jiff
- **jiff-tzdb** 0.1.8 — https://github.com/BurntSushi/jiff
- **jiff-tzdb-platform** 0.1.3 — https://github.com/BurntSushi/jiff
- **memchr** 2.8.3 — https://github.com/BurntSushi/memchr
- **winapi-util** 0.1.11 — https://github.com/BurntSushi/winapi-util

### Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT

- **linux-raw-sys** 0.12.1 — https://github.com/sunfishcode/linux-raw-sys
- **linux-raw-sys** 0.4.15 — https://github.com/sunfishcode/linux-raw-sys
- **rustix** 0.38.44 — https://github.com/bytecodealliance/rustix
- **rustix** 1.1.4 — https://github.com/bytecodealliance/rustix
- **wasi** 0.11.1+wasi-snapshot-preview1 — https://github.com/bytecodealliance/wasi
- **wasip2** 1.0.4+wasi-0.2.12 — https://github.com/bytecodealliance/wasi-rs
- **wit-bindgen** 0.57.1 — https://github.com/bytecodealliance/wit-bindgen

### BSD-3-Clause

- **alloc-no-stdlib** 2.0.4 — https://github.com/dropbox/rust-alloc-no-stdlib
- **alloc-stdlib** 0.2.4 — https://github.com/dropbox/rust-alloc-no-stdlib
- **curve25519-dalek** 4.1.3 — https://github.com/dalek-cryptography/curve25519-dalek/tree/main/curve25519-dalek
- **ed25519-dalek** 2.2.0 — https://github.com/dalek-cryptography/curve25519-dalek/tree/main/ed25519-dalek
- **subtle** 2.6.1 — https://github.com/dalek-cryptography/subtle

### ISC

- **forwarded-header-value** 0.1.1 — https://github.com/EasyPost/rust-forwarded-header-value
- **rustls-webpki** 0.103.13 — https://github.com/rustls/webpki
- **simple_asn1** 0.6.4 — https://github.com/acw/simple_asn1
- **untrusted** 0.9.0 — https://github.com/briansmith/untrusted

### Apache-2.0 OR ISC OR MIT

- **hyper-rustls** 0.27.9 — https://github.com/rustls/hyper-rustls
- **rustls** 0.23.42 — https://github.com/rustls/rustls
- **rustls-native-certs** 0.8.4 — https://github.com/rustls/rustls-native-certs

### BSD-2-Clause

- **arrayref** 0.3.9 — https://github.com/droundy/arrayref
- **kamadak-exif** 0.6.1 — https://github.com/kamadak/exif-rs
- **mutate_once** 0.1.2 — https://github.com/kamadak/mutate_once-rs

### MIT OR Apache-2.0 OR Zlib

- **tinyvec_macros** 0.1.1 — https://github.com/Soveu/tinyvec_macros
- **zune-core** 0.5.1 — https://github.com/etemesi254/zune-image
- **zune-jpeg** 0.5.15 — https://github.com/etemesi254/zune-image/tree/dev/crates/zune-jpeg

### Apache-2.0/MIT

- **bytes-utils** 0.1.4 — https://github.com/vorner/bytes-utils
- **crc32c** 0.6.8 — https://github.com/zowens/crc32c

### BSD-2-Clause OR Apache-2.0 OR MIT

- **zerocopy** 0.8.55 — https://github.com/google/zerocopy
- **zerocopy-derive** 0.8.55 — https://github.com/google/zerocopy

### BSD-3-Clause OR Apache-2.0

- **moxcms** 0.8.1 — https://github.com/awxkee/moxcms.git
- **pxfm** 0.1.30 — https://github.com/awxkee/pxfm

### CC0-1.0 OR MIT-0 OR Apache-2.0

- **constant_time_eq** 0.4.2 — https://github.com/cesarb/constant_time_eq
- **dunce** 1.0.5 — https://gitlab.com/kornelski/dunce

### MIT OR Apache-2.0 OR LGPL-2.1-or-later

- **r-efi** 5.3.0 — https://github.com/r-efi/r-efi
- **r-efi** 6.0.0 — https://github.com/r-efi/r-efi

### Unlicense/MIT

- **same-file** 1.0.6 — https://github.com/BurntSushi/same-file
- **walkdir** 2.5.0 — https://github.com/BurntSushi/walkdir

### Zlib OR Apache-2.0 OR MIT

- **bytemuck** 1.25.2 — https://github.com/Lokathor/bytemuck
- **tinyvec** 1.12.0 — https://github.com/Lokathor/tinyvec

### (Apache-2.0 OR MIT) AND BSD-3-Clause

- **encoding_rs** 0.8.35 — https://github.com/hsivonen/encoding_rs

### (MIT OR Apache-2.0) AND Apache-2.0

- **moka** 0.12.15 — https://github.com/moka-rs/moka

### (MIT OR Apache-2.0) AND Unicode-3.0

- **unicode-ident** 1.0.24 — https://github.com/dtolnay/unicode-ident

### 0BSD OR MIT OR Apache-2.0

- **adler2** 2.0.1 — https://github.com/oyvindln/adler2

### Apache-2.0 / MIT

- **fnv** 1.0.7 — https://github.com/servo/rust-fnv

### Apache-2.0 AND ISC

- **ring** 0.17.14 — https://github.com/briansmith/ring

### Apache-2.0 OR BSL-1.0

- **ryu** 1.0.23 — https://github.com/dtolnay/ryu

### BSD-3-Clause AND MIT

- **brotli** 8.0.4 — https://github.com/dropbox/rust-brotli

### BSD-3-Clause/MIT

- **brotli-decompressor** 5.0.3 — https://github.com/dropbox/rust-brotli-decompressor

### CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception

- **blake3** 1.8.5 — https://github.com/BLAKE3-team/BLAKE3

### CDLA-Permissive-2.0

- **webpki-root-certs** 1.0.9 — https://github.com/rustls/webpki-roots

### MIT AND BSD-3-Clause

- **matchit** 0.8.4 — https://github.com/ibraheemdev/matchit

### MIT OR Apache-2.0 OR BSD-1-Clause

- **fiat-crypto** 0.2.9 — https://github.com/mit-plv/fiat-crypto

### MIT OR Zlib OR Apache-2.0

- **miniz_oxide** 0.8.9 — https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide

### Zlib

- **foldhash** 0.2.0 — https://github.com/orlp/foldhash

---

Full license texts are distributed with each crate in the Cargo registry and are
available at the repositories listed above. To regenerate this file after a
dependency change, re-run the generator or use `cargo about`.

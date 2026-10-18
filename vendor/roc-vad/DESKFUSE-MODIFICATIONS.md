# DeskFuse microphone driver

This directory is derived from Roc VAD v0.0.4, commit
`19d621a8364b0d6f44969c340bdc59fe891981c8`. Covered files remain MPL-2.0;
see LICENSE and AUTHORS.md. Original repository: https://github.com/roc-streaming/roc-vad.

DeskFuse modifications:

- Private bundle and device identities are loaded from the microphone section of
  `config/previous-installation.json` to preserve compatibility with installed drivers.
  UUID `E88452E3-1C2E-42C1-A1A4-7B496BC3D527` remains independent of upstream Roc VAD.
- Control endpoint is fixed to loopback `127.0.0.1:19712`. Only the private
  receiver is accepted, with loopback RTP/RS8M/RTCP endpoints 19721–19723.
  External endpoints and virtual speaker creation are rejected.
- Receiver reset RPC drops Roc sessions and cached HAL samples without removing
  the device. Reader status reflects first StartIO through last StopIO.
- Bound HAL history reads and handle full-ring copies, including requests larger
  than the history buffer; missed history returns silence.
- Errors read as silence, driver logging is metadata-only, 48 kHz mono RTP uses
  payload type 96. The application manages authenticated network transport.
- Resolve C/C++ compilers and SDK from the same selected Xcode; invalidate
  only owned build caches when the compiler/SDK fingerprint changes.
- Product builds exclude test targets and fix all dependency commits. Archive
  SHA256 values and Roc transitive distribution archives are in sources.lock.json.
- Build compatibility: use system SDK libz; preserve Abseil paired Apple
  architecture arguments and remove an obsolete gRPC template disambiguator
  using deterministic, checksummed build patches.
- Order every protocol-header consumer after RPC generation for clean parallel builds.
- Native C ABI sender/RPC shim lives in apps/deskfuse-macos/MicrophoneNative.

Build from repository root using `scripts/build-roc-microphone.sh arm64`.
The source bundle includes these modifications and all compiled dependencies,
including their original license notices. It contains no installed driver and
performs no installation. Build tools needed: Apple clang/SDK, cmake, scons,
pkg-config, autoconf, automake, libtool and Go.

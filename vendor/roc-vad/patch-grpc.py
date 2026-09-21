#!/usr/bin/env python3
"""Preserve paired architecture flags on modern CMake/Apple clang (build only)."""
import pathlib
import sys
source=pathlib.Path(sys.argv[1])/'third_party/abseil-cpp/absl/copts/AbseilConfigureCopts.cmake'
text=source.read_text()
old='list(APPEND ABSL_RANDOM_RANDEN_COPTS "-Xarch_${_arch}" "${_flag}")'
new='list(APPEND ABSL_RANDOM_RANDEN_COPTS "SHELL:-Xarch_${_arch} ${_flag}")'
if old in text: source.write_text(text.replace(old,new))
elif new not in text: raise SystemExit('Pinned Abseil architecture flag patch does not match')
source=pathlib.Path(sys.argv[1])/'src/core/lib/promise/detail/basic_seq.h'
text=source.read_text()
old='Traits::template CallSeqFactory(f_, *cur_, std::move(arg))'
new='Traits::CallSeqFactory(f_, *cur_, std::move(arg))'
if old in text: source.write_text(text.replace(old,new))
elif new not in text: raise SystemExit('Pinned gRPC template compatibility patch does not match')

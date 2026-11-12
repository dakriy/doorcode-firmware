# Shared environment for the verification scripts. Sourced, not executed.
# cargo-installed tools live in ~/.cargo/bin, which is not on PATH on every
# machine. cargo-llvm-cov finds llvm-cov and llvm-profdata in the
# toolchain's llvm-tools component; a toolchain without it uses the system
# LLVM tools. Override the paths if yours differ.
export PATH="$HOME/.cargo/bin:$PATH"
if [ ! -x "$(rustc --print target-libdir)/../bin/llvm-cov" ]; then
    export LLVM_COV="${LLVM_COV:-/usr/bin/llvm-cov}"
    export LLVM_PROFDATA="${LLVM_PROFDATA:-/usr/bin/llvm-profdata}"
fi

# Code that is measured and mutated: application code only.
APPLICATION_SRC="crates/app/src crates/protocol/src"
# Everything the coverage report ignores: connector interfaces, test support,
# connector implementations, production wiring, tests, and the cargo registry.
COVERAGE_IGNORE='(crates/(connectors|testkit|connectors-host|host|swarm)/|/tests/|/\.cargo/registry/|/rustc/)'

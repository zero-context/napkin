Static binaries. No toolchain, no dependencies.

```sh
# linux x86_64 — swap for napkin-aarch64-linux (Pi 5, Ampere, Graviton)
# or napkin-macos-universal (Apple silicon and Intel)
curl -fsSL https://github.com/__REPO__/releases/latest/download/napkin-x86_64-linux -o napkin
chmod +x napkin
sudo cpupower frequency-set -g performance   # linux only; macos needs no step
./napkin
```

It writes one JSON file describing your machine. Open a pull request with that
file and your hardware is in the dataset. If your machine is not listed yet,
that is the interesting case — unusual hardware is worth more here than fast
hardware.

napkin refuses to write a result if the machine is busy, if the clock is not
pinned, or if it slows down mid-run. A refusal is the tool working.

Building from source still works, and is the route on RISC-V and the BSDs for
now: `cargo build --release`.

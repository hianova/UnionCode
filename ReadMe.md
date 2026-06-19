# UnionCode

[![Crates.io](https://img.shields.io/crates/v/union_code.svg)](https://crates.io/crates/union_code)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Build Status](https://img.shields.io/badge/build-passing-brightgreen)](#)
[![no_std](https://img.shields.io/badge/no__std-compatible-success.svg)](#)
[![loom](https://img.shields.io/badge/loom-verified-blue)](#)

**UnionCode** is an extreme-compression semantic router designed for embedded systems, edge devices, and ultra-high-performance server nodes. Functioning as a deterministic "Intent Engine," it transforms noisy, colloquial human text—such as voice transcriptions—into a minimal `3-byte` actionable binary payload (`CompressedIntent`).

Engineered strictly for high assurance and predictable execution constraints, UnionCode provides **zero heap allocations**, **zero mutexes/locks**, and operates purely through `&self` immutable, wait-free concurrent pipelines.

## Core Features

- **Extreme Binary Compression**: Normalizes unstructured semantic language inputs (e.g., `"hey, can you unlock the delivery box for me please?"`) into a deterministic `3-byte` payload composed of an `OpCode` and a `PayloadID`.
- **Zero Allocation (`#![no_std]`)**: Operates entirely via statically-allocated RAM buffers and Read-Only Memory (ROM) execution. 100% compatible with Cortex-M, Xtensa (ESP32), and bare-metal environments.
- **Lock-Free Concurrency**: The underlying Fast-State Transducer (FST) revolves around purely immutable `&[u8]` arrays, enabling multi-threaded execution over a single `Arc<UnionCode>` reference without race conditions or memory leaks.
- **Plug-and-Play Dictionaries**: Vocabularies are authored as flat CSV/text dictionaries and pre-compiled at build time (`build.rs`) into an Aho-Corasick Finite State Transducer (FST) static ROM. Dynamic routing is achieved by simply swapping the static matrix.

## Architecture

UnionCode employs a sophisticated, zero-allocation pipeline:

1. **FST Routing (O(N))**: The engine systematically iterates through the compiled deterministic FST static matrix. Successful matches are automatically hoisted and returned as a minimal tuple. Unmatched strings return a configurable `0x06 (NotFound)` code.

## Quick Start Usage

Add UnionCode to your project's `Cargo.toml`:

```toml
[dependencies]
union_code = "0.2.0"
```

### 1. Define Dictionaries

Create flat text files under the `dictionaries/` directory at the root of your project (e.g., `dictionaries/default.txt`).
Format instructions as `[KIND],[KEYWORD],[HEX_CODE]`.

```csv
VERB,打開,0x01
VERB,解鎖,0x01
NOUN,箱子,0x0000
NOUN,櫃子,0x0000
```

### 2. Embedded Implementation Example

UnionCode generates a fast, immutable ROM matrix at build time. Include it and initialize your pipeline:

```rust
use union_code::{FstEngine, UnionCode, CompressedIntent};

// Inject the statically compiled ROM Matrix directly into binary flash
include!(concat!(env!("OUT_DIR"), "/default_rom.rs"));

fn main() {
    // Initialize the FST Engine and validate its integrity against corruption
    let fst = FstEngine::new(DEFAULT_ROM_MATRIX);
    assert!(fst.validate_rom(), "Corrupted Static ROM Detected");
    
    // Construct the primary UnionCode translator
    let uc = UnionCode::new(fst);
    
    // Process colloquial language deterministically
    let input = "欸那個，幫我把箱子打開一下啦，謝囉";
    if let Ok(intent) = uc.decode(input.as_bytes()) {
        println!("OpCode: 0x{:02X}", intent.opcode);         // Outputs: 0x01
        println!("PayloadID: 0x{:04X}", intent.payload_id);  // Outputs: 0x0000
    }
}
```

## Security & Reliability

UnionCode has undergone rigorous code audits to verify system safety:
- **`#[repr(C)]` Compliance**: Data structures conform strictly to C-compatible layouts to prevent undefined behavior (UB) and misaligned byte access.
- **ROM Validation**: The `validate_rom(&self)` method prevents arbitrary out-of-bounds pointer execution common with malicious or corrupted FST arrays.


## Performance Profiles

Metrics obtained on modern M-Series hardware under `--release` conditions.

| Component | Metric | Description |
|-----------|--------|-------------|
| **FST String Traversal** | `~幾十奈秒` | Uncached DFA character branching sequence |

## License

MIT License. Supported and continuously enhanced by Gemini 3.1 Pro.

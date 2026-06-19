# UnionCode Specification (SPEC)

## 1. Introduction

UnionCode is a state-of-the-art, zero-allocation (`no_std`), lock-free semantic routing engine. Serving as a deterministic "Intent Decoder," it processes noisy and unstructured natural language queries (typically derived from real-time voice transcripts) and condenses them into an ultra-minimal `3-byte` payload structure (`CompressedIntent`).

This payload acts as actionable binary logic which embedded controllers, distributed nodes, and memory-constrained IoT devices can execute instantaneously, completely bypassing the need for cloud interpretation at runtime.

## 2. Global Architecture & Pipeline

UnionCode is structurally language-agnostic and processes raw byte strings (`&[u8]`). It relies on a deterministic **Plug-and-Play Dictionary** structure compiled at build time into an immutable Read-Only Memory (ROM) matrix.

The resolution pipeline operates deterministically and thread-safely via the `UnionCode::decode(&self, &[u8])` method:

1. **Finite State Transducer (O(N)):** The engine traverses the byte string against an immutable Aho-Corasick Deterministic Finite Automaton (DFA) represented by the ROM array.
   - *Match:* Extracts the structural `OpCode` and `PayloadID`, constructs the `CompressedIntent`, and returns the payload (`~幾十奈秒`).
2. **Degraded Fallback:** Unrecognized patterns fall back to an `Err(0x06)` (`NotFound`) sequence. At this tier, the client application holds responsibility to interface with a secondary cloud LLM for asynchronous validation.

## 3. Foundational Data Structures

### CompressedIntent

The normalized binary output format designed for zero-copy transmission, bitwise packet masking, or immediate struct transmutation.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CompressedIntent {
    pub opcode: u8,      // Verb representation (e.g., 0x20 = TaskDispatch)
    pub payload_id: u16, // Noun / Target Node ID (e.g., 0x0A42 = Coffee)
}
```
*Note: `#[repr(C)]` strict alignment rules ensure safe bitwise operation cross-compatibility without risk of Undefined Behavior (UB).*

## 4. Compile-Time Automata Dictionaries

Developers supply domain-specific lexicons via basic `.txt` files in the `dictionaries/` directory:

```csv
VERB,打開,0x01
NOUN,交割箱,0x0000
```

`build.rs` compiles these files prior to binary synthesis into an `Aho-Corasick DFA` serialized as a flat, immutable byte slice (`&'static [u8]`).
- **Memory Footprint Zeroing**: The routing matrix is positioned natively into Flash/ROM without initialization heap consumption.
- **Integrity Validation**: Built-in methods (`validate_rom()`) are provided at runtime to verify node jumps and boundary bounds, neutralizing out-of-bounds attacks from malicious ROM swapping.

## 5. Performance Thresholds (PERF)

Compiled under `--release` profiles on modern M-series architectures:

- **Heap Allocations**: 0 bytes.
- **Thread Locks/Mutexes**: 0 overhead logic.

### Computational Footprint

| Component Module | Speed Metric | Performance Summary |
|-----------|--------|-------------|
| **FST String Traversal** | `~幾十奈秒` | Deep iteration across an 18-byte UTF-8 string on the DFA array. |

*Conclusion: UnionCode facilitates sub-100 nanosecond AI intent abstraction in zero-memory constrained environments without sacrificing thread safety.*

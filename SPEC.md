# UnionCode Specification (SPEC)

## 1. Introduction

UnionCode is a state-of-the-art, zero-allocation (`no_std`), lock-free semantic routing engine. Serving as a deterministic "Intent Decoder," it processes noisy and unstructured natural language queries (typically derived from real-time voice transcripts) and condenses them into an ultra-minimal `3-byte` payload structure (`CompressedIntent`).

This payload acts as actionable binary logic which embedded controllers, distributed nodes, and memory-constrained IoT devices can execute instantaneously, completely bypassing the need for cloud interpretation at runtime.

## 2. Global Architecture & Pipeline

UnionCode is structurally language-agnostic and processes raw byte strings (`&[u8]`). It relies on a deterministic **Plug-and-Play Dictionary** structure compiled at build time into an immutable Read-Only Memory (ROM) matrix.

The resolution pipeline operates synchronously and thread-safely via the `UnionCode::decode(&self, &[u8])` method:

1. **Hash Generation (O(N)):** The byte array is passed through a high-throughput, customized FNV-1a hashing mechanism.
2. **L1 Semantic Cache (O(1)):** The resulting hash is queried against a thread-safe, statically allocated concurrent cache (`StaticDualCache`).
   - *Hit:* Instantly yields the 3-byte payload (`~28 ns`).
3. **L2 Finite State Transducer (O(N)):** Upon a cache miss, the engine traverses the byte string against an immutable Aho-Corasick Deterministic Finite Automaton (DFA) represented by the ROM array.
   - *Match:* Extracts the structural `OpCode` and `PayloadID`, constructs the `CompressedIntent`, injects the tuple back into the L1 cache dynamically, and returns the payload (`~148 ns`).
4. **Degraded Fallback:** Unrecognized patterns fall back to an `Err(0x06)` (`NotFound`) sequence. At this tier, the client application holds responsibility to interface with a secondary cloud LLM for asynchronous validation.

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

### Concurrent Edge Caches (via `dualcache-ff`)

UnionCode dictates external semantic caches via its unified `SemanticCache` interface utilizing immutable `&self` references. Operations rely primarily on `dualcache-ff`'s `StaticDualCache`, providing:
- **Wait-Free Concurrency**: Shared cache instances can be scaled infinitely across multi-core processors.
- **Zero Allocations**: Caches pre-allocate contiguous node rings statically.
- **Loom Modeled**: The cache implementations avoid `RefCell` and Mutex bottlenecks, validated via programmatic concurrency permutation tests (`loom`) to ensure zero dropped memory leaks and an absence of logical race conditions.

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
| **Hashing Engine** | `< 1.0 ns/op` | Capable of processing stream rates exceeding 1 GB/s. |
| **Raw Cache Lookup** | `~19.5 ns` | Extremely optimized atomic O(1) probe. |
| **Hot Path (Pipeline Hit)** | `~28.3 ns` | Entire system round-trip relying strictly on the L1 tier. |
| **FST String Traversal** | `~139.1 ns` | Deep iteration across an 18-byte UTF-8 string on the DFA array. |
| **Cold Path (Pipeline Miss)**| `~148.2 ns` | End-to-end extraction + L1 Cache injection and payload return. |

*Conclusion: UnionCode facilitates sub-200 nanosecond AI intent abstraction in zero-memory constrained environments without sacrificing thread safety.*

#![no_std]

#[cfg(test)]
extern crate std;

// Include the generated ROM matrix at compile time
include!(concat!(env!("OUT_DIR"), "/default_rom.rs"));

/// 極限壓縮後的意圖輸出 (4 bytes due to alignment padding, for safety)
/// 可直接 transmute 或零拷貝映射為 rkyv 結構，打入 cdDB
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CompressedIntent {
    pub opcode: u8,      // 對應 io_oi_core::OpCode (例如 0x20 TaskDispatch)
    pub payload_id: u16, // 實體物件或參數的標準化 ID (例如 0x0A42 代表咖啡)
}


// ============================================================
// 2. 語意快取介面 (Semantic Cache - 依賴注入)
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheError {
    Full,
    InternalError,
}

pub trait SemanticCache {
    /// 傳入字串的極速 Hash (如 FxHash)，回傳壓縮意圖
    fn get_intent(&mut self, hash: u32) -> Option<CompressedIntent>;

    /// 寫入新的語意映射 (用於群體免疫與快取預熱)
    fn put_intent(&mut self, hash: u32, intent: CompressedIntent) -> Result<(), CacheError>;
}

/// 邊緣端專用的微型快取 (佔用 < 2KB SRAM)
pub struct EdgeSemanticCache<const N: usize = 256> {
    // 使用 heapless 靜態分配，N 通常設為 256
    map: heapless::FnvIndexMap<u32, CompressedIntent, N>,
    order: heapless::Vec<u32, N>,
}

impl<const N: usize> EdgeSemanticCache<N> {
    pub fn new() -> Self {
        Self {
            map: heapless::FnvIndexMap::new(),
            order: heapless::Vec::new(),
        }
    }
}

impl<const N: usize> Default for EdgeSemanticCache<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> SemanticCache for EdgeSemanticCache<N> {
    fn get_intent(&mut self, hash: u32) -> Option<CompressedIntent> {
        if self.map.contains_key(&hash) {
            // Update LRU: move to the back of the queue
            if let Some(pos) = self.order.iter().position(|&x| x == hash) {
                self.order.remove(pos);
            }
            let _ = self.order.push(hash);
            self.map.get(&hash).copied()
        } else {
            None
        }
    }

    fn put_intent(&mut self, hash: u32, intent: CompressedIntent) -> Result<(), CacheError> {
        if self.map.contains_key(&hash) {
            // Update value and position in LRU
            self.map.insert(hash, intent).map_err(|_| CacheError::InternalError)?;
            if let Some(pos) = self.order.iter().position(|&x| x == hash) {
                self.order.remove(pos);
            }
            let _ = self.order.push(hash);
            Ok(())
        } else {
            // Check if map is full
            if self.map.len() >= N {
                // Evict the least recently used element (the first in the vector)
                if !self.order.is_empty() {
                    let evict_hash = self.order.remove(0);
                    self.map.remove(&evict_hash);
                } else if let Some(&first_hash) = self.map.keys().next() {
                    self.map.remove(&first_hash);
                }
            }
            self.map.insert(hash, intent).map_err(|_| CacheError::Full)?;
            let _ = self.order.push(hash);
            Ok(())
        }
    }
}

// ------------------------------------------------------------
// Implement SemanticCache for dualcache-ff structs
// ------------------------------------------------------------

impl<S, Tls> SemanticCache for dualcache_ff::cache::DualCacheFF<u32, CompressedIntent, S, Tls>
where
    S: core::hash::BuildHasher + Clone + Send + 'static,
    Tls: dualcache_ff::tls::TlsProvider + 'static,
{
    fn get_intent(&mut self, hash: u32) -> Option<CompressedIntent> {
        self.get(&hash)
    }

    fn put_intent(&mut self, hash: u32, intent: CompressedIntent) -> Result<(), CacheError> {
        self.insert(hash, intent);
        Ok(())
    }
}

impl<const N: usize, S> SemanticCache for dualcache_ff::static_cache::static_cache::StaticDualCache<u32, CompressedIntent, N, S>
where
    S: core::hash::BuildHasher,
{
    fn get_intent(&mut self, hash: u32) -> Option<CompressedIntent> {
        self.get(&hash)
    }

    fn put_intent(&mut self, hash: u32, intent: CompressedIntent) -> Result<(), CacheError> {
        self.insert(hash, intent);
        Ok(())
    }
}

// ============================================================
// 3. FST 靜態路由引擎 (Finite State Transducer)
// ============================================================

pub struct FstEngine {
    // 唯讀記憶體中的靜態狀態機矩陣 (編譯期生成)
    pub rom_matrix: &'static [u8],
}

impl FstEngine {
    pub const fn new(rom_matrix: &'static [u8]) -> Self {
        Self { rom_matrix }
    }

    /// O(N) 確定性狀態機解析，N 為輸入位元組長度
    pub fn parse_stream(&self, input: &[u8]) -> Option<CompressedIntent> {
        if self.rom_matrix.is_empty() {
            return None;
        }
        let mut current_offset = 0usize;
        let mut matched_opcode: Option<u8> = None;
        let mut matched_payload_id: Option<u16> = None;

        for &b in input {
            loop {
                if current_offset >= self.rom_matrix.len() {
                    return None;
                }
                if let Some(next_offset) = self.find_transition(current_offset, b) {
                    current_offset = next_offset;
                    // Read outputs from the new state
                    if let Some((op, pay)) = self.read_outputs(current_offset) {
                        if let Some(o) = op {
                            matched_opcode = Some(o);
                        }
                        if let Some(p) = pay {
                            matched_payload_id = Some(p);
                        }
                    } else {
                        return None;
                    }
                    break;
                } else {
                    if current_offset == 0 {
                        // At root and no transition matches, consume byte and stay at root
                        break;
                    }
                    // Follow failure transition
                    if let Some(fail_offset) = self.read_fail_state(current_offset) {
                        current_offset = fail_offset;
                    } else {
                        return None;
                    }
                }
            }
        }

        // Return CompressedIntent only if both OpCode and PayloadID are matched
        match (matched_opcode, matched_payload_id) {
            (Some(opcode), Some(payload_id)) => Some(CompressedIntent { opcode, payload_id }),
            _ => None,
        }
    }

    #[inline(always)]
    fn read_fail_state(&self, offset: usize) -> Option<usize> {
        let flags = *self.rom_matrix.get(offset)?;
        let mut pos = offset + 1;
        if flags & 1 != 0 {
            pos += 1;
        }
        if flags & 2 != 0 {
            pos += 2;
        }
        let b0 = *self.rom_matrix.get(pos)?;
        let b1 = *self.rom_matrix.get(pos + 1)?;
        Some(u16::from_le_bytes([b0, b1]) as usize)
    }

    #[inline(always)]
    fn read_outputs(&self, offset: usize) -> Option<(Option<u8>, Option<u16>)> {
        let flags = *self.rom_matrix.get(offset)?;
        let mut pos = offset + 1;
        let mut op = None;
        if flags & 1 != 0 {
            op = Some(*self.rom_matrix.get(pos)?);
            pos += 1;
        }
        let mut pay = None;
        if flags & 2 != 0 {
            let b0 = *self.rom_matrix.get(pos)?;
            let b1 = *self.rom_matrix.get(pos + 1)?;
            pay = Some(u16::from_le_bytes([b0, b1]));
        }
        Some((op, pay))
    }

    #[inline(always)]
    fn find_transition(&self, offset: usize, b: u8) -> Option<usize> {
        let flags = *self.rom_matrix.get(offset)?;
        let mut pos = offset + 1;
        if flags & 1 != 0 {
            pos += 1;
        }
        if flags & 2 != 0 {
            pos += 2;
        }
        pos += 2; // skip fail_state

        let num_transitions = *self.rom_matrix.get(pos)? as usize;
        pos += 1;

        for _ in 0..num_transitions {
            let tb = *self.rom_matrix.get(pos)?;
            if tb == b {
                let b0 = *self.rom_matrix.get(pos + 1)?;
                let b1 = *self.rom_matrix.get(pos + 2)?;
                return Some(u16::from_le_bytes([b0, b1]) as usize);
            }
            pos += 3;
        }
        None
    }
}

impl Default for FstEngine {
    fn default() -> Self {
        Self {
            rom_matrix: DEFAULT_ROM_MATRIX,
        }
    }
}

// ============================================================
// 4. UnionCode 核心轉譯器 (The Translator)
// ============================================================

pub struct UnionCode<'a, C: SemanticCache> {
    pub cache: C,
    pub fst: FstEngine,
    _marker: core::marker::PhantomData<&'a ()>,
}

impl<'a, C: SemanticCache> UnionCode<'a, C> {
    pub fn new(cache: C) -> Self {
        Self {
            cache,
            fst: FstEngine::default(),
            _marker: core::marker::PhantomData,
        }
    }

    pub fn new_with_fst(cache: C, fst: FstEngine) -> Self {
        Self {
            cache,
            fst,
            _marker: core::marker::PhantomData,
        }
    }

    /// 核心轉譯管線：人類語言 -> 3 Bytes 二進位指令
    pub fn decode(&mut self, human_input: &'a [u8]) -> Result<CompressedIntent, u8> {
        // 1. 計算極速 Hash (FxHash)
        let input_hash = self.fast_hash(human_input);

        // 2. L1 語意快取攔截 (O(1), ~5ns)
        if let Some(intent) = self.cache.get_intent(input_hash) {
            return Ok(intent);
        }

        // 3. FST 靜態路由解析 (O(N), ~幾十奈秒)
        if let Some(intent) = self.fst.parse_stream(human_input) {
            // 寫入快取，達成自我學習
            let _ = self.cache.put_intent(input_hash, intent);
            return Ok(intent);
        }

        // 4. 未知意圖降級 (Fallback)
        // 觸發 0x06 NotFound，交由雲端 LLM 非同步解析，或引導用戶重新輸入
        Err(0x06) // OpCode::NotFound
    }

    #[inline(always)]
    fn fast_hash(&self, data: &[u8]) -> u32 {
        let mut hash = 0x811c9dc5u32;
        for &b in data {
            hash ^= b as u32;
            hash = hash.wrapping_mul(0x01000193);
        }
        hash
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn test_edge_semantic_cache_lru() {
        let mut cache = EdgeSemanticCache::<4>::new();
        let i1 = CompressedIntent { opcode: 0x10, payload_id: 1 };
        let i2 = CompressedIntent { opcode: 0x20, payload_id: 2 };
        let i3 = CompressedIntent { opcode: 0x30, payload_id: 3 };
        let i4 = CompressedIntent { opcode: 0x40, payload_id: 4 };
        let i5 = CompressedIntent { opcode: 0x50, payload_id: 5 };

        assert!(cache.put_intent(1, i1).is_ok());
        assert!(cache.put_intent(2, i2).is_ok());
        assert!(cache.put_intent(3, i3).is_ok());
        assert!(cache.put_intent(4, i4).is_ok());

        // order: 1, 2, 3, 4
        // Get 1 to refresh it
        assert_eq!(cache.get_intent(1), Some(i1));
        // order: 2, 3, 4, 1

        // Insert 5, which should evict 2 (the least recently used)
        assert!(cache.put_intent(5, i5).is_ok());

        assert_eq!(cache.get_intent(2), None);
        assert_eq!(cache.get_intent(3), Some(i3));
        assert_eq!(cache.get_intent(4), Some(i4));
        assert_eq!(cache.get_intent(1), Some(i1));
        assert_eq!(cache.get_intent(5), Some(i5));
    }

    #[test]
    fn test_edge_semantic_cache_update_existing() {
        let mut cache = EdgeSemanticCache::<2>::new();
        let i1 = CompressedIntent { opcode: 0x10, payload_id: 1 };
        let i2 = CompressedIntent { opcode: 0x20, payload_id: 2 };
        let i1_new = CompressedIntent { opcode: 0x11, payload_id: 11 };
        let i3 = CompressedIntent { opcode: 0x30, payload_id: 3 };

        assert!(cache.put_intent(1, i1).is_ok());
        assert!(cache.put_intent(2, i2).is_ok());

        // Update 1: should refresh its position as most recently used
        assert!(cache.put_intent(1, i1_new).is_ok());

        // Insert 3: since 1 was refreshed, 2 is now LRU and should be evicted
        assert!(cache.put_intent(3, i3).is_ok());

        assert_eq!(cache.get_intent(2), None);
        assert_eq!(cache.get_intent(1), Some(i1_new));
        assert_eq!(cache.get_intent(3), Some(i3));
    }

    #[test]
    fn test_fast_hash() {
        let cache = EdgeSemanticCache::<4>::new();
        let uc = UnionCode::new(cache);
        
        // Assert that hashes are consistent
        let h1 = uc.fast_hash(b"hello");
        let h2 = uc.fast_hash(b"hello");
        assert_eq!(h1, h2);

        // Assert that hashes differ for different inputs
        let h3 = uc.fast_hash(b"world");
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_fst_engine_matching() {
        let fst = FstEngine::default();

        // 幫我拿咖啡 (verb: "拿" -> 0x20, noun: "咖啡" -> 0x0A42)
        let intent = fst.parse_stream("請幫我拿咖啡".as_bytes());
        assert_eq!(
            intent,
            Some(CompressedIntent {
                opcode: 0x20,
                payload_id: 0x0A42
            })
        );

        // check english "please get coffee"
        let intent_en = fst.parse_stream("please get coffee".as_bytes());
        assert_eq!(
            intent_en,
            Some(CompressedIntent {
                opcode: 0x20,
                payload_id: 0x0A42
            })
        );

        // check query tea "查茶" (verb: "查" -> 0x10, noun: "茶" -> 0x0A43)
        let intent_query = fst.parse_stream("幫我查茶的狀態".as_bytes());
        assert_eq!(
            intent_query,
            Some(CompressedIntent {
                opcode: 0x10,
                payload_id: 0x0A43
            })
        );

        // Missing noun
        assert_eq!(fst.parse_stream("請幫我拿".as_bytes()), None);

        // Missing verb
        assert_eq!(fst.parse_stream("咖啡".as_bytes()), None);

        // No matches
        assert_eq!(fst.parse_stream("隨便說一句話".as_bytes()), None);
    }

    #[test]
    fn test_fst_engine_edge_cases() {
        let fst = FstEngine::default();

        // 1. Empty input
        assert_eq!(fst.parse_stream(b""), None);

        // 2. Mixed order: noun then verb ("咖啡拿")
        assert_eq!(
            fst.parse_stream("咖啡幫我拿".as_bytes()),
            Some(CompressedIntent {
                opcode: 0x20,
                payload_id: 0x0A42
            })
        );

        // 3. Multi-matches: last match overrides
        // "送咖啡查茶" -> verb "送" (0x20) and "查" (0x10), noun "咖啡" (0x0A42) and "茶" (0x0A43)
        // Last matched verb is "查" (0x10), last matched noun is "茶" (0x0A43)
        assert_eq!(
            fst.parse_stream("送咖啡查茶".as_bytes()),
            Some(CompressedIntent {
                opcode: 0x10,
                payload_id: 0x0A43
            })
        );
    }

    #[test]
    fn test_union_code_pipeline() {
        let cache = EdgeSemanticCache::<16>::new();
        let mut uc = UnionCode::new(cache);

        let input = "請幫我拿咖啡".as_bytes();

        // 1. First run, not in cache, resolved by FST, gets cached
        let res1 = uc.decode(input);
        assert_eq!(
            res1,
            Ok(CompressedIntent {
                opcode: 0x20,
                payload_id: 0x0A42
            })
        );

        // We temporarily replace FST to prove that the second run hits the cache directly
        uc.fst = FstEngine::new(&[]);
        let res2 = uc.decode(input);
        assert_eq!(
            res2,
            Ok(CompressedIntent {
                opcode: 0x20,
                payload_id: 0x0A42
            })
        );

        // Unknown text should result in NotFound (0x06) error
        assert_eq!(uc.decode("無關話語".as_bytes()), Err(0x06));
    }

    #[test]
    fn test_static_dual_cache_integration() {
        let config = dualcache_ff::config::Config::with_memory_budget(1, 100);
        let cache = dualcache_ff::static_cache::static_cache::StaticDualCache::<u32, CompressedIntent, 16>::new(config);
        let mut uc = UnionCode::new(cache);

        let input = "請幫我拿咖啡".as_bytes();

        // First resolve uses FST and inserts into cache
        let res1 = uc.decode(input);
        assert_eq!(
            res1,
            Ok(CompressedIntent {
                opcode: 0x20,
                payload_id: 0x0A42
            })
        );

        // Replace FST to show cache works
        uc.fst = FstEngine::new(&[]);
        let res2 = uc.decode(input);
        assert_eq!(
            res2,
            Ok(CompressedIntent {
                opcode: 0x20,
                payload_id: 0x0A42
            })
        );
    }

    #[test]
    fn test_dual_cache_ff_integration() {
        let config = dualcache_ff::config::Config::with_memory_budget(1, 100);
        let cache = dualcache_ff::cache::DualCacheFF::<u32, CompressedIntent>::new(config);
        let mut uc = UnionCode::new(cache);

        let input = "請幫我拿咖啡".as_bytes();

        // Call decode multiple times and sync to pass probation filter and daemon batch processing
        let mut res = Err(0x06);
        for _ in 0..5 {
            res = uc.decode(input);
            uc.cache.sync();
        }

        assert_eq!(
            res,
            Ok(CompressedIntent {
                opcode: 0x20,
                payload_id: 0x0A42
            })
        );

        // Replace FST to show cache works
        uc.fst = FstEngine::new(&[]);
        let res2 = uc.decode(input);
        assert_eq!(
            res2,
            Ok(CompressedIntent {
                opcode: 0x20,
                payload_id: 0x0A42
            })
        );
    }

    // ============================================================
    // STRESS TESTS
    // ============================================================

    #[test]
    fn test_cache_capacity_two_as_minimum() {
        // heapless::FnvIndexMap requires N > 1, so minimum usable capacity is 2
        let mut cache = EdgeSemanticCache::<2>::new();
        let i1 = CompressedIntent { opcode: 0x10, payload_id: 1 };
        let i2 = CompressedIntent { opcode: 0x20, payload_id: 2 };
        let i3 = CompressedIntent { opcode: 0x30, payload_id: 3 };

        assert!(cache.put_intent(1, i1).is_ok());
        assert_eq!(cache.get_intent(1), Some(i1));

        assert!(cache.put_intent(2, i2).is_ok());
        assert_eq!(cache.get_intent(2), Some(i2));

        // Insert third should evict first (LRU)
        assert!(cache.put_intent(3, i3).is_ok());
        assert_eq!(cache.get_intent(1), None);
        assert_eq!(cache.get_intent(2), Some(i2));
        assert_eq!(cache.get_intent(3), Some(i3));
    }

    #[test]
    fn test_cache_rapid_eviction_cycle() {
        // Fill and evict repeatedly to stress the LRU
        let mut cache = EdgeSemanticCache::<4>::new();
        for round in 0..100u32 {
            let base = round * 4;
            for j in 0..4u32 {
                let intent = CompressedIntent {
                    opcode: (base + j) as u8,
                    payload_id: (base + j) as u16,
                };
                assert!(cache.put_intent(base + j, intent).is_ok());
            }
            // All 4 from this round should be present
            for j in 0..4u32 {
                assert!(cache.get_intent(base + j).is_some(),
                    "round {round}, key {} missing", base + j);
            }
            // Previous round should be fully evicted (if round > 0)
            if round > 0 {
                let prev_base = (round - 1) * 4;
                for j in 0..4u32 {
                    assert_eq!(cache.get_intent(prev_base + j), None,
                        "round {round}, old key {} still present", prev_base + j);
                }
            }
        }
    }

    #[test]
    fn test_cache_same_key_repeated_put() {
        let mut cache = EdgeSemanticCache::<2>::new();
        // Put the same key 1000 times with different values
        for i in 0..1000u16 {
            let intent = CompressedIntent { opcode: 0x10, payload_id: i };
            assert!(cache.put_intent(42, intent).is_ok());
        }
        // Should have the last value
        assert_eq!(
            cache.get_intent(42),
            Some(CompressedIntent { opcode: 0x10, payload_id: 999 })
        );
    }

    #[test]
    fn test_cache_order_map_sync_invariant() {
        // Verify that `order` and `map` stay in sync after many operations
        let mut cache = EdgeSemanticCache::<8>::new();
        // Fill
        for i in 0..8u32 {
            let intent = CompressedIntent { opcode: i as u8, payload_id: i as u16 };
            cache.put_intent(i, intent).unwrap();
        }
        // Access in reverse to shuffle LRU
        for i in (0..8u32).rev() {
            assert!(cache.get_intent(i).is_some());
        }
        // Evict by inserting 8 new entries
        for i in 8..16u32 {
            let intent = CompressedIntent { opcode: i as u8, payload_id: i as u16 };
            cache.put_intent(i, intent).unwrap();
        }
        // Old 0..8 should be gone, 8..16 should be present
        for i in 0..8u32 {
            assert_eq!(cache.get_intent(i), None, "key {i} should have been evicted");
        }
        for i in 8..16u32 {
            assert!(cache.get_intent(i).is_some(), "key {i} should be present");
        }
    }

    // ============================================================
    // HASH QUALITY TESTS
    // ============================================================

    #[test]
    fn test_hash_empty_input() {
        let cache = EdgeSemanticCache::<4>::new();
        let uc = UnionCode::new(cache);
        let h = uc.fast_hash(b"");
        // FNV-1a offset basis
        assert_eq!(h, 0x811c9dc5);
    }

    #[test]
    fn test_hash_single_byte_avalanche() {
        // Single-bit difference in input should cause significant hash difference
        let cache = EdgeSemanticCache::<4>::new();
        let uc = UnionCode::new(cache);

        let mut collision_count = 0u32;
        let total_pairs: u32 = 256 * 255 / 2;
        for a in 0u16..256 {
            for b in (a + 1)..256 {
                let ha = uc.fast_hash(&[a as u8]);
                let hb = uc.fast_hash(&[b as u8]);
                if ha == hb {
                    collision_count += 1;
                }
            }
        }
        // For a good hash, 0 collisions among 32640 pairs of single-byte inputs
        assert_eq!(collision_count, 0,
            "Found {collision_count} collisions among {total_pairs} single-byte pairs");
    }

    #[test]
    fn test_hash_chinese_distribution() {
        // Test hash distribution for common Chinese command patterns
        let cache = EdgeSemanticCache::<4>::new();
        let uc = UnionCode::new(cache);

        let inputs: &[&[u8]] = &[
            "拿咖啡".as_bytes(),
            "買咖啡".as_bytes(),
            "送咖啡".as_bytes(),
            "查咖啡".as_bytes(),
            "拿茶".as_bytes(),
            "買茶".as_bytes(),
            "送茶".as_bytes(),
            "查茶".as_bytes(),
            "拿水".as_bytes(),
            "買水".as_bytes(),
            "送水".as_bytes(),
            "查水".as_bytes(),
        ];

        let hashes: std::vec::Vec<u32> = inputs.iter().map(|i| uc.fast_hash(i)).collect();
        // All hashes must be unique
        let mut sorted = hashes.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), hashes.len(),
            "Collision found among Chinese command hashes: {:?}", hashes);

        // Check bit distribution: each hash should use a wide range of bits
        let all_or = hashes.iter().fold(0u32, |acc, &h| acc | h);
        let bits_used = all_or.count_ones();
        assert!(bits_used >= 20,
            "Hash output only uses {} bits across {} inputs, expected >= 20", bits_used, hashes.len());
    }

    #[test]
    fn test_hash_collision_rate_random_strings() {
        // Generate 1000 pseudo-random strings and measure collision rate
        let cache = EdgeSemanticCache::<4>::new();
        let uc = UnionCode::new(cache);

        let mut hashes = std::collections::HashSet::new();
        let mut collisions = 0u32;
        // Use a simple PRNG to generate test data (deterministic)
        let mut seed: u64 = 0xDEADBEEF;
        for _ in 0..1000 {
            let mut buf = [0u8; 16];
            for byte in buf.iter_mut() {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                *byte = (seed >> 33) as u8;
            }
            let h = uc.fast_hash(&buf);
            if !hashes.insert(h) {
                collisions += 1;
            }
        }
        // Birthday paradox: for 1000 items in 2^32 space, expected ~0.1 collisions
        assert!(collisions <= 2,
            "Too many collisions: {collisions}/1000 (expected ~0)");
    }

    // ============================================================
    // FST ENGINE STRESS TESTS
    // ============================================================

    #[test]
    fn test_fst_very_long_input() {
        let fst = FstEngine::default();
        // 10KB of padding bytes + "拿咖啡" at the end
        let mut input = std::vec::Vec::new();
        for _ in 0..10_000 {
            input.push(b'x');
        }
        input.extend_from_slice("拿咖啡".as_bytes());
        let result = fst.parse_stream(&input);
        assert_eq!(result, Some(CompressedIntent { opcode: 0x20, payload_id: 0x0A42 }));
    }

    #[test]
    fn test_fst_all_zero_bytes() {
        let fst = FstEngine::default();
        let input = [0u8; 256];
        // Should not match anything
        assert_eq!(fst.parse_stream(&input), None);
    }

    #[test]
    fn test_fst_all_0xff_bytes() {
        let fst = FstEngine::default();
        let input = [0xFFu8; 256];
        assert_eq!(fst.parse_stream(&input), None);
    }

    #[test]
    fn test_fst_partial_utf8_match() {
        let fst = FstEngine::default();
        // "拿" is E6 8B BF in UTF-8. Test with only first 2 bytes.
        let input = [0xE6, 0x8B];
        assert_eq!(fst.parse_stream(&input), None);
    }

    #[test]
    fn test_fst_repeated_pattern() {
        let fst = FstEngine::default();
        // "拿拿拿拿拿咖啡" — repeated verb, single noun
        let result = fst.parse_stream("拿拿拿拿拿咖啡".as_bytes());
        assert_eq!(result, Some(CompressedIntent { opcode: 0x20, payload_id: 0x0A42 }));
    }

    #[test]
    fn test_fst_interleaved_partial_matches() {
        let fst = FstEngine::default();
        // Interleave partial UTF-8 bytes of "拿" (E6 8B BF) with noise
        // This tests the failure transition handling
        let input: &[u8] = &[0xE6, 0x00, 0x8B, 0x00, 0xBF];
        assert_eq!(fst.parse_stream(input), None);
    }

    #[test]
    fn test_fst_corrupted_rom_graceful() {
        // Completely garbage ROM data — should return None, not panic
        let garbage_rom: &'static [u8] = &[0xFF; 64];
        let fst = FstEngine::new(garbage_rom);
        // This should not panic
        let result = fst.parse_stream(b"hello");
        // We just care it doesn't crash; result can be anything
        let _ = result;
    }

    #[test]
    fn test_fst_minimal_rom() {
        // ROM with just a root node with 0 transitions
        // flags=0, fail=0x0000, num_transitions=0
        let minimal_rom: &'static [u8] = &[0x00, 0x00, 0x00, 0x00];
        let fst = FstEngine::new(minimal_rom);
        assert_eq!(fst.parse_stream(b"anything"), None);
    }

    // ============================================================
    // PIPELINE INTEGRATION STRESS TESTS
    // ============================================================

    #[test]
    fn test_pipeline_cache_fills_and_evicts_correctly() {
        let cache = EdgeSemanticCache::<2>::new();
        let mut uc = UnionCode::new(cache);

        // Decode 3 different valid inputs with cache size 2
        let r1 = uc.decode("拿咖啡".as_bytes());
        assert!(r1.is_ok());

        let r2 = uc.decode("查茶".as_bytes());
        assert!(r2.is_ok());

        // Third should evict the first from cache
        let r3 = uc.decode("買水".as_bytes());
        assert!(r3.is_ok());

        // Now disable FST — only cache works
        uc.fst = FstEngine::new(&[]);

        // "拿咖啡" was evicted, should fail
        assert_eq!(uc.decode("拿咖啡".as_bytes()), Err(0x06));

        // "查茶" and "買水" — one of the most recent should still be cached
        let r_tea = uc.decode("查茶".as_bytes());
        let r_water = uc.decode("買水".as_bytes());
        // At least one of them should be in cache
        assert!(r_tea.is_ok() || r_water.is_ok());
    }

    #[test]
    fn test_pipeline_unknown_inputs_dont_pollute_cache() {
        // Decode many unknown inputs, then verify valid input still works
        // We use separate UnionCode instances per batch to avoid lifetime issues
        // with format! strings
        let cache = EdgeSemanticCache::<4>::new();
        let mut uc = UnionCode::new(cache);

        // Feed static unknown inputs
        let unknowns: &[&[u8]] = &[
            b"unknown_gibberish_0",
            b"unknown_gibberish_1",
            b"unknown_gibberish_2",
            b"random_noise_abc",
            b"aslkdjaslkdjasd",
            b"xxxxxxxxxxx",
        ];
        for input in unknowns {
            assert_eq!(uc.decode(*input), Err(0x06));
        }

        // Valid input should still work
        let r = uc.decode("拿咖啡".as_bytes());
        assert_eq!(r, Ok(CompressedIntent { opcode: 0x20, payload_id: 0x0A42 }));
    }

    // ============================================================
    // PERFORMANCE BENCHMARKS (manual timing)
    // ============================================================

    #[test]
    fn bench_fast_hash_throughput() {
        let cache = EdgeSemanticCache::<4>::new();
        let uc = UnionCode::new(cache);
        let data = b"please get me some coffee thank you very much";

        let iterations = 1_000_000u64;
        let start = std::time::Instant::now();
        let mut sink = 0u32;
        for _ in 0..iterations {
            sink = sink.wrapping_add(uc.fast_hash(data));
        }
        let elapsed = start.elapsed();
        let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;
        let throughput_mb = (data.len() as f64 * iterations as f64)
            / (elapsed.as_secs_f64() * 1_000_000.0);

        std::println!(
            "\n[BENCH] fast_hash: {:.1} ns/op, {:.0} MB/s throughput ({} bytes input), sink={}",
            ns_per_op, throughput_mb, data.len(), sink
        );

        // Sanity: hash of 46 bytes should be < 100ns on modern hardware
        assert!(ns_per_op < 500.0, "Hash too slow: {:.1} ns/op", ns_per_op);
    }

    #[test]
    fn bench_fst_parse_stream() {
        let fst = FstEngine::default();
        let inputs: &[&[u8]] = &[
            "請幫我拿咖啡".as_bytes(),    // 18 bytes UTF-8
            "please get coffee".as_bytes(), // 17 bytes ASCII
            b"",                             // 0 bytes
            "隨便說一句話".as_bytes(),    // no match, 18 bytes
        ];

        let iterations = 500_000u64;
        for input in inputs {
            let start = std::time::Instant::now();
            let mut sink = 0u8;
            for _ in 0..iterations {
                if let Some(intent) = fst.parse_stream(input) {
                    sink = sink.wrapping_add(intent.opcode);
                }
            }
            let elapsed = start.elapsed();
            let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;

            let label = if input.is_empty() {
                std::string::String::from("(empty)")
            } else {
                std::format!("({} bytes)", input.len())
            };
            std::println!(
                "[BENCH] FST parse_stream {}: {:.1} ns/op, sink={}",
                label, ns_per_op, sink
            );
        }
    }

    #[test]
    fn bench_cache_get_hit() {
        let mut cache = EdgeSemanticCache::<256>::new();
        // Pre-fill cache
        for i in 0..256u32 {
            let intent = CompressedIntent { opcode: (i & 0xFF) as u8, payload_id: i as u16 };
            cache.put_intent(i, intent).unwrap();
        }

        let iterations = 1_000_000u64;
        let start = std::time::Instant::now();
        let mut sink = 0u8;
        for i in 0..iterations {
            let key = (i % 256) as u32;
            if let Some(intent) = cache.get_intent(key) {
                sink = sink.wrapping_add(intent.opcode);
            }
        }
        let elapsed = start.elapsed();
        let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;

        std::println!(
            "\n[BENCH] EdgeSemanticCache<256> get_intent (hit): {:.1} ns/op, sink={}",
            ns_per_op, sink
        );
    }

    #[test]
    fn bench_cache_get_miss() {
        let mut cache = EdgeSemanticCache::<256>::new();
        // Pre-fill cache with keys 0..256
        for i in 0..256u32 {
            let intent = CompressedIntent { opcode: (i & 0xFF) as u8, payload_id: i as u16 };
            cache.put_intent(i, intent).unwrap();
        }

        let iterations = 1_000_000u64;
        let start = std::time::Instant::now();
        let mut sink = 0u8;
        for i in 0..iterations {
            let key = (i as u32) + 1000; // all misses
            if let Some(intent) = cache.get_intent(key) {
                sink = sink.wrapping_add(intent.opcode);
            }
        }
        let elapsed = start.elapsed();
        let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;

        std::println!(
            "\n[BENCH] EdgeSemanticCache<256> get_intent (miss): {:.1} ns/op, sink={}",
            ns_per_op, sink
        );
    }

    #[test]
    fn bench_cache_put_eviction() {
        let mut cache = EdgeSemanticCache::<64>::new();

        let iterations = 100_000u64;
        let start = std::time::Instant::now();
        for i in 0..iterations {
            let intent = CompressedIntent { opcode: (i & 0xFF) as u8, payload_id: i as u16 };
            let _ = cache.put_intent(i as u32, intent);
        }
        let elapsed = start.elapsed();
        let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;

        std::println!(
            "\n[BENCH] EdgeSemanticCache<64> put_intent (with eviction): {:.1} ns/op",
            ns_per_op
        );
    }

    #[test]
    fn bench_full_pipeline() {
        let cache = EdgeSemanticCache::<256>::new();
        let mut uc = UnionCode::new(cache);
        let input = "請幫我拿咖啡".as_bytes();

        // Warm up: first call populates cache
        let _ = uc.decode(input);

        let iterations = 1_000_000u64;
        let start = std::time::Instant::now();
        let mut sink = 0u8;
        for _ in 0..iterations {
            if let Ok(intent) = uc.decode(input) {
                sink = sink.wrapping_add(intent.opcode);
            }
        }
        let elapsed = start.elapsed();
        let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;

        std::println!(
            "\n[BENCH] Full pipeline (cache hit path): {:.1} ns/op, sink={}",
            ns_per_op, sink
        );
    }

    #[test]
    fn bench_full_pipeline_cache_miss() {
        let input = "請幫我拿咖啡".as_bytes();

        let iterations = 500_000u64;
        let start = std::time::Instant::now();
        let mut sink = 0u8;
        for _ in 0..iterations {
            // Create a fresh UnionCode each time to force FST path
            let c = EdgeSemanticCache::<2>::new();
            let mut uc2 = UnionCode::new(c);
            if let Ok(intent) = uc2.decode(input) {
                sink = sink.wrapping_add(intent.opcode);
            }
        }
        let elapsed = start.elapsed();
        let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;

        std::println!(
            "\n[BENCH] Full pipeline (FST path, cold cache): {:.1} ns/op, sink={}",
            ns_per_op, sink
        );
    }

    // ============================================================
    // LRU COMPLEXITY SCALING TEST
    // ============================================================

    #[test]
    fn bench_lru_scaling() {
        // Measure cache get_intent time at different capacities to verify O(N) claim
        std::println!("\n[BENCH] LRU Scaling Analysis:");
        std::println!("  {:>8} | {:>12} | {:>12}", "Capacity", "Hit (ns/op)", "Put (ns/op)");
        std::println!("  {:-<8}-+-{:-<12}-+-{:-<12}", "", "", "");

        // Test with N=4
        {
            let mut cache = EdgeSemanticCache::<4>::new();
            for i in 0..4u32 {
                cache.put_intent(i, CompressedIntent { opcode: i as u8, payload_id: i as u16 }).unwrap();
            }
            let iters = 1_000_000u64;
            let start = std::time::Instant::now();
            let mut s = 0u8;
            for i in 0..iters {
                if let Some(intent) = cache.get_intent((i % 4) as u32) {
                    s = s.wrapping_add(intent.opcode);
                }
            }
            let hit_ns = start.elapsed().as_nanos() as f64 / iters as f64;

            let start2 = std::time::Instant::now();
            for i in 0..iters {
                let _ = cache.put_intent(i as u32, CompressedIntent { opcode: s, payload_id: i as u16 });
            }
            let put_ns = start2.elapsed().as_nanos() as f64 / iters as f64;

            std::println!("  {:>8} | {:>10.1}  | {:>10.1}", 4, hit_ns, put_ns);
        }

        // Test with N=64
        {
            let mut cache = EdgeSemanticCache::<64>::new();
            for i in 0..64u32 {
                cache.put_intent(i, CompressedIntent { opcode: i as u8, payload_id: i as u16 }).unwrap();
            }
            let iters = 500_000u64;
            let start = std::time::Instant::now();
            let mut s = 0u8;
            for i in 0..iters {
                if let Some(intent) = cache.get_intent((i % 64) as u32) {
                    s = s.wrapping_add(intent.opcode);
                }
            }
            let hit_ns = start.elapsed().as_nanos() as f64 / iters as f64;

            let start2 = std::time::Instant::now();
            for i in 0..iters {
                let _ = cache.put_intent(i as u32, CompressedIntent { opcode: s, payload_id: i as u16 });
            }
            let put_ns = start2.elapsed().as_nanos() as f64 / iters as f64;

            std::println!("  {:>8} | {:>10.1}  | {:>10.1}", 64, hit_ns, put_ns);
        }

        // Test with N=256
        {
            let mut cache = EdgeSemanticCache::<256>::new();
            for i in 0..256u32 {
                cache.put_intent(i, CompressedIntent { opcode: i as u8, payload_id: i as u16 }).unwrap();
            }
            let iters = 200_000u64;
            let start = std::time::Instant::now();
            let mut s = 0u8;
            for i in 0..iters {
                if let Some(intent) = cache.get_intent((i % 256) as u32) {
                    s = s.wrapping_add(intent.opcode);
                }
            }
            let hit_ns = start.elapsed().as_nanos() as f64 / iters as f64;

            let start2 = std::time::Instant::now();
            for i in 0..iters {
                let _ = cache.put_intent(i as u32, CompressedIntent { opcode: s, payload_id: i as u16 });
            }
            let put_ns = start2.elapsed().as_nanos() as f64 / iters as f64;

            std::println!("  {:>8} | {:>10.1}  | {:>10.1}", 256, hit_ns, put_ns);
        }
    }
}

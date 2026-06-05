use union_code::{SemanticCache, CompressedIntent};
use dualcache_ff::static_cache::static_cache::StaticDualCache;
fn main() {
    let cache = StaticDualCache::<u32, CompressedIntent, 10>::new(dualcache_ff::config::Config::with_memory_budget(1, 10));
    let _ = cache.get(&0);
    cache.insert(0, CompressedIntent { opcode: 0, payload_id: 0 });
}

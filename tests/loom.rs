
use union_code::{UnionCode, CompressedIntent};
use dualcache_ff::static_cache::static_cache::StaticDualCache;
use dualcache_ff::config::Config;
use loom::sync::Arc;
use loom::thread;

#[test]
fn test_loom_concurrency_no_leak_thread_safe() {
    loom::model(|| {
        let config = Config::with_memory_budget(1, 100);
        let cache = StaticDualCache::<u32, CompressedIntent, 4>::new(config);
        
        let uc = UnionCode::new(cache);
        let uc_arc = Arc::new(uc);
        
        let mut threads = vec![];
        
        for i in 0..3 {
            let uc_clone = uc_arc.clone();
            threads.push(thread::spawn(move || {
                let input = if i % 2 == 0 {
                    "請幫我拿咖啡".as_bytes()
                } else {
                    "查茶".as_bytes()
                };
                
                // Concurrent lock-free access via &self
                let res = uc_clone.decode(input);
                if i % 2 == 0 {
                    assert_eq!(res, Ok(CompressedIntent { opcode: 0x20, payload_id: 0x0A42 }));
                } else {
                    assert_eq!(res, Ok(CompressedIntent { opcode: 0x10, payload_id: 0x0A43 }));
                }
            }));
        }
        
        for t in threads {
            t.join().unwrap();
        }
        
        // Assert that the Arc reference count drops correctly.
        // Loom tracks this and ensures memory leaks don't happen.
    });
}

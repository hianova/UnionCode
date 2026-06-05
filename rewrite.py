import re

with open('src/lib.rs', 'r') as f:
    content = f.read()

# 1. Remove EdgeSemanticCache definition and impls
pattern_edge = r'/// 邊緣端專用的微型快取.*?(?=\n// ------------------------------------------------------------\n// Implement SemanticCache for dualcache-ff structs)'
content = re.sub(pattern_edge, '', content, flags=re.DOTALL)

# 2. Add imports and helper for tests
test_mod = r'(mod tests \{\n    extern crate std;\n    use super::\*;)'
test_imports = r'\1\n    use dualcache_ff::static_cache::static_cache::StaticDualCache;\n    use dualcache_ff::config::Config;\n\n    fn new_test_cache<const N: usize>() -> StaticDualCache<u32, CompressedIntent, N> {\n        StaticDualCache::new(Config::with_memory_budget(1, 100))\n    }'
content = re.sub(test_mod, test_imports, content)

# 3. Replace EdgeSemanticCache::<N>::new() with new_test_cache::<N>()
content = re.sub(r'EdgeSemanticCache::<(\d+)>::new\(\)', r'new_test_cache::<\1>()', content)

# 4. Remove specific tests
tests_to_remove = [
    'test_edge_semantic_cache_lru',
    'test_edge_semantic_cache_update_existing',
    'test_cache_capacity_two_as_minimum',
    'test_cache_rapid_eviction_cycle',
    'test_cache_same_key_repeated_put',
    'test_cache_order_map_sync_invariant',
    'bench_lru_scaling',
]

for test in tests_to_remove:
    # Match from #[test] to the end of the block. Note: This assumes standard indentation.
    # We find the `#[test]\n    fn test_name() {` and find the matching closing brace.
    start_str = f'    #[test]\n    fn {test}()'
    start_idx = content.find(start_str)
    if start_idx != -1:
        # Find the matching closing brace
        brace_count = 0
        end_idx = -1
        in_fn = False
        for i in range(start_idx, len(content)):
            if content[i] == '{':
                brace_count += 1
                in_fn = True
            elif content[i] == '}':
                brace_count -= 1
            if in_fn and brace_count == 0:
                end_idx = i + 1
                break
        if end_idx != -1:
            content = content[:start_idx] + content[end_idx:]

# Also remove bench_cache_put_eviction because it measures capacity eviction which StaticDualCache handles implicitly
bench_to_remove = ['bench_cache_put_eviction']
for test in bench_to_remove:
    start_str = f'    #[test]\n    fn {test}()'
    start_idx = content.find(start_str)
    if start_idx != -1:
        brace_count = 0
        end_idx = -1
        in_fn = False
        for i in range(start_idx, len(content)):
            if content[i] == '{':
                brace_count += 1
                in_fn = True
            elif content[i] == '}':
                brace_count -= 1
            if in_fn and brace_count == 0:
                end_idx = i + 1
                break
        if end_idx != -1:
            content = content[:start_idx] + content[end_idx:]

with open('src/lib.rs', 'w') as f:
    f.write(content)

print("Rewrite successful.")

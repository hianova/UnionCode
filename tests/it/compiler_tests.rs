use union_code::compiler::compile_rom;
use union_code::{FstEngine, CompressedIntent};

#[test]
fn test_compiler_round_trip() {
    let verbs = vec![("拿", 0x20), ("查", 0x10), ("get", 0x30)];
    let nouns = vec![("咖啡", 0x0A42), ("茶", 0x0A43), ("water", 0x0A44)];
    
    let rom = compile_rom(&verbs, &nouns);
    
    // FstEngine expects a &'static [u8], so we leak the compiled ROM for testing
    let static_rom: &'static [u8] = Box::leak(rom.into_boxed_slice());
    
    let fst = FstEngine::new(static_rom);
    
    // Validate ROM integrity
    assert!(fst.validate_rom(), "Compiled ROM failed structural validation");
    
    // Test basic parsing
    let intent1 = fst.parse_stream("幫我拿咖啡".as_bytes());
    assert_eq!(intent1, Some(CompressedIntent { opcode: 0x20, payload_id: 0x0A42 }));
    
    let intent2 = fst.parse_stream("請查茶".as_bytes());
    assert_eq!(intent2, Some(CompressedIntent { opcode: 0x10, payload_id: 0x0A43 }));
    
    let intent3 = fst.parse_stream("please get water".as_bytes());
    assert_eq!(intent3, Some(CompressedIntent { opcode: 0x30, payload_id: 0x0A44 }));
    
    // Test missing noun
    let intent_missing = fst.parse_stream("幫我拿".as_bytes());
    assert_eq!(intent_missing, None);
}

#[test]
fn test_compiler_edge_cases() {
    let verbs = vec![("a", 0x01)];
    let nouns = vec![("b", 0x0002)];
    let rom = compile_rom(&verbs, &nouns);
    let static_rom: &'static [u8] = Box::leak(rom.into_boxed_slice());
    let fst = FstEngine::new(static_rom);
    
    assert!(fst.validate_rom());
    assert_eq!(fst.parse_stream("ab".as_bytes()), Some(CompressedIntent { opcode: 0x01, payload_id: 0x0002 }));
    assert_eq!(fst.parse_stream("ba".as_bytes()), Some(CompressedIntent { opcode: 0x01, payload_id: 0x0002 }));
}

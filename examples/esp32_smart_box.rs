use union_code::{FstEngine, UnionCode, CompressedIntent};


// Load the compiled plug-and-play dictionary for the Smart Box application.
// This is generated at compile time from `dictionaries/smart_box.txt`.
include!(concat!(env!("OUT_DIR"), "/smart_box_rom.rs"));

fn main() {
    println!("=== UnionCode Reference Implementation: ESP32 Smart Box ===");
    println!("This demo showcases the extreme decoupling and 'Plug-and-Play' nature of UnionCode.\n");
    
    // 1. Initialize the FST Engine with our domain-specific ROM Matrix.
    // The SMART_BOX_ROM_MATRIX is a zero-allocation static slice representing
    // our exact dictionary (verbs like 打開/解鎖 and nouns like 箱子/交割箱).
    let fst = FstEngine::new(SMART_BOX_ROM_MATRIX);
    
    // 2. Assemble the UnionCode Engine
    let uc = UnionCode::new(fst);
    
    // 4. The Magic Moment: An extremely colloquial, noisy user input.
    let input = "欸那個，幫我把交割箱打開一下啦，謝囉";
    println!("[Input]  User Voice Transcription: \"{}\"", input);
    
    // 5. Decode the input in O(N) time without allocating memory.
    let start_time = std::time::Instant::now();
    let result = uc.decode(input.as_bytes());
    let elapsed = start_time.elapsed();
    
    println!("[Engine] Processing time: {:?}", elapsed);
    
    match result {
        Ok(intent) => {
            println!("\n✨ MAGIC MOMENT ✨");
            println!("Noisy natural language was sliced down to just 3 bytes!");
            println!("[Output] OpCode:    0x{:02X} (e.g. Action: Toggle Relay)", intent.opcode);
            println!("[Output] PayloadID: 0x{:04X} (e.g. Target: Lock 0)", intent.payload_id);
            
            // Prove it matches the expected [0x01, 0x00, 0x00] byte format
            let bytes = unsafe { core::mem::transmute::<CompressedIntent, [u8; 4]>(intent) };
            println!("[Output] Hex Dump:  [{:02X}, {:02X}, {:02X}] (Ignoring padding)", bytes[0], bytes[1], bytes[2]);
            println!("\nThe ESP32 toggles the GPIO pin in < 1ms. 喀！");
        }
        Err(e) => {
            println!("Unknown Intent, Fallback Code: 0x{:02X}", e);
        }
    }
}

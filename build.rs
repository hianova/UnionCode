use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::fs::File;
use std::io::Write;
use std::path::Path;

struct Node {
    transitions: BTreeMap<u8, usize>,
    fail: usize,
    opcode: Option<u8>,
    payload_id: Option<u16>,
}

fn compile_rom(verbs: &[(&str, u8)], nouns: &[(&str, u16)]) -> Vec<u8> {
    let mut nodes = vec![Node {
        transitions: BTreeMap::new(),
        fail: 0,
        opcode: None,
        payload_id: None,
    }];

    // Helper to insert a string pattern
    let mut insert_pattern = |pattern: &str, opcode: Option<u8>, payload_id: Option<u16>| {
        let mut curr = 0;
        for &b in pattern.as_bytes() {
            curr = if let Some(&next) = nodes[curr].transitions.get(&b) {
                next
            } else {
                let next = nodes.len();
                nodes.push(Node {
                    transitions: BTreeMap::new(),
                    fail: 0,
                    opcode: None,
                    payload_id: None,
                });
                nodes[curr].transitions.insert(b, next);
                next
            };
        }
        if opcode.is_some() {
            nodes[curr].opcode = opcode;
        }
        if payload_id.is_some() {
            nodes[curr].payload_id = payload_id;
        }
    };

    // Insert all verbs
    for &(verb, opcode) in verbs {
        insert_pattern(verb, Some(opcode), None);
    }

    // Insert all nouns
    for &(noun, payload_id) in nouns {
        insert_pattern(noun, None, Some(payload_id));
    }

    // Compute failure transitions (BFS)
    let mut queue = VecDeque::new();
    // Depth 1 nodes
    let root_transitions = nodes[0].transitions.clone();
    for (&_b, &child) in &root_transitions {
        nodes[child].fail = 0;
        queue.push_back(child);
    }

    while let Some(curr) = queue.pop_front() {
        let curr_transitions = nodes[curr].transitions.clone();
        let fail_state = nodes[curr].fail;

        for (&b, &child) in &curr_transitions {
            let mut f = fail_state;
            loop {
                if let Some(&next) = nodes[f].transitions.get(&b) {
                    nodes[child].fail = next;
                    break;
                }
                if f == 0 {
                    nodes[child].fail = 0;
                    break;
                }
                f = nodes[f].fail;
            }
            queue.push_back(child);
        }
    }

    // Calculate serialization offsets
    let mut offsets = vec![0; nodes.len()];
    let mut total_size = 0;
    for (i, node) in nodes.iter().enumerate() {
        offsets[i] = total_size;
        let mut size = 1; // flags
        if node.opcode.is_some() {
            size += 1;
        }
        if node.payload_id.is_some() {
            size += 2;
        }
        size += 2; // fail
        size += 1; // num_transitions
        size += 3 * node.transitions.len();
        total_size += size;
    }

    // Serialize
    let mut buf = vec![0; total_size];
    for (i, node) in nodes.iter().enumerate() {
        let offset = offsets[i];
        let mut flags = 0u8;
        if node.opcode.is_some() {
            flags |= 1;
        }
        if node.payload_id.is_some() {
            flags |= 2;
        }
        buf[offset] = flags;
        
        let mut pos = offset + 1;
        if let Some(op) = node.opcode {
            buf[pos] = op;
            pos += 1;
        }
        if let Some(pay) = node.payload_id {
            let bytes = pay.to_le_bytes();
            buf[pos] = bytes[0];
            buf[pos + 1] = bytes[1];
            pos += 2;
        }

        let fail_offset = offsets[node.fail] as u16;
        let fail_bytes = fail_offset.to_le_bytes();
        buf[pos] = fail_bytes[0];
        buf[pos + 1] = fail_bytes[1];
        pos += 2;

        buf[pos] = node.transitions.len() as u8;
        pos += 1;

        for (&b, &child) in &node.transitions {
            buf[pos] = b;
            let child_offset = offsets[child] as u16;
            let child_bytes = child_offset.to_le_bytes();
            buf[pos + 1] = child_bytes[0];
            buf[pos + 2] = child_bytes[1];
            pos += 3;
        }
    }

    buf
}

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dict_dir = Path::new("dictionaries");
    
    if dict_dir.exists() {
        println!("cargo:rerun-if-changed=dictionaries");

        for entry in std::fs::read_dir(dict_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("txt") {
                let stem = path.file_stem().unwrap().to_str().unwrap();
                let dest_path = Path::new(&out_dir).join(format!("{}_rom.rs", stem));
                
                let content = std::fs::read_to_string(&path).unwrap();
                
                let mut verbs = Vec::new();
                let mut nouns = Vec::new();
                
                for line in content.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    let parts: Vec<&str> = line.split(',').collect();
                    if parts.len() >= 3 {
                        let word = parts[1];
                        let hex_str = parts[2].trim_start_matches("0x");
                        match parts[0] {
                            "VERB" => {
                                if let Ok(code) = u8::from_str_radix(hex_str, 16) {
                                    verbs.push((word, code));
                                }
                            }
                            "NOUN" => {
                                if let Ok(code) = u16::from_str_radix(hex_str, 16) {
                                    nouns.push((word, code));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                
                let bytes = compile_rom(&verbs, &nouns);
                let mut f = File::create(&dest_path).unwrap();
                
                let const_name = format!("{}_ROM_MATRIX", stem.to_uppercase());
                writeln!(f, "pub const {}: &[u8] = &{:?};", const_name, bytes).unwrap();
                
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
    
    println!("cargo:rerun-if-changed=build.rs");
}

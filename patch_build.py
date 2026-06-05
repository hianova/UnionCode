import re

with open('build.rs', 'r') as f:
    content = f.read()

new_main = """fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dict_dir = Path::new("dictionaries");
    
    if dict_dir.exists() {
        println!("cargo:rerun-if-changed=dictionaries");

        let mut grouped_files: std::collections::HashMap<String, Vec<std::path::PathBuf>> = std::collections::HashMap::new();

        fn visit_dirs(dir: &Path, grouped: &mut std::collections::HashMap<String, Vec<std::path::PathBuf>>) {
            if dir.is_dir() {
                for entry in std::fs::read_dir(dir).unwrap() {
                    let entry = entry.unwrap();
                    let path = entry.path();
                    if path.is_dir() {
                        visit_dirs(&path, grouped);
                    } else if path.extension().and_then(|s| s.to_str()) == Some("txt") {
                        let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
                        grouped.entry(stem).or_default().push(path);
                    }
                }
            }
        }

        visit_dirs(dict_dir, &mut grouped_files);

        for (stem, paths) in grouped_files {
            let dest_path = Path::new(&out_dir).join(format!("{}_rom.rs", stem));
            
            let mut entries: Vec<(String, Option<u8>, Option<u16>)> = Vec::new();
            
            for path in &paths {
                let content = std::fs::read_to_string(path).unwrap();
                for line in content.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    let parts: Vec<&str> = line.split(',').collect();
                    if parts.len() >= 3 {
                        let word = parts[1].trim();
                        let hex_str = parts[2].trim().trim_start_matches("0x");
                        match parts[0].trim() {
                            "VERB" => {
                                if let Ok(code) = u8::from_str_radix(hex_str, 16) {
                                    entries.push((word.to_string(), Some(code), None));
                                }
                            }
                            "NOUN" => {
                                if let Ok(code) = u16::from_str_radix(hex_str, 16) {
                                    entries.push((word.to_string(), None, Some(code)));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            
            let bytes = compile_rom(&entries);
            let mut f = File::create(&dest_path).unwrap();
            
            let const_name = format!("{}_ROM_MATRIX", stem.to_uppercase());
            writeln!(f, "pub const {}: &[u8] = &{:?};", const_name, bytes).unwrap();
            
            for path in paths {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
    
    println!("cargo:rerun-if-changed=build.rs");
}
"""

main_idx = content.find('fn main() {')
if main_idx != -1:
    new_content = content[:main_idx] + new_main
    with open('build.rs', 'w') as f:
        f.write(new_content)
    print("Patched successfully")
else:
    print("Could not find fn main()")

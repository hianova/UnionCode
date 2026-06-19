#![no_std]

#[cfg(test)]
extern crate std;

extern crate alloc;

// Include the generated ROM matrix at compile time
pub mod compiler;
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
// 3. FST 靜態路由引擎 (Finite State Transducer)
// ============================================================

/// `FstEngine` 是一個基於 Aho-Corasick 自動機的靜態路由引擎。
/// 它透過 O(N) 的時間複雜度解析位元組串流，無任何記憶體分配。
///
/// # Examples
/// ```
/// use union_code::{FstEngine, CompressedIntent};
/// 
/// // 使用預設的靜態 ROM 矩陣
/// let fst = FstEngine::default();
/// 
/// // 確保 ROM 矩陣完好無損
/// assert!(fst.validate_rom());
/// 
/// // 解析輸入
/// let result = fst.parse_stream("請幫我拿咖啡".as_bytes());
/// // assert_eq!(result, Some(CompressedIntent { opcode: 0x20, payload_id: 0x0A42 }));
/// ```
pub struct FstEngine {
    // 唯讀記憶體中的靜態狀態機矩陣 (編譯期生成)
    pub rom_matrix: &'static [u8],
}

impl FstEngine {
    pub const fn new(rom_matrix: &'static [u8]) -> Self {
        Self { rom_matrix }
    }

    /// 驗證 ROM 矩陣的結構完整性，確保所有的狀態轉移指標皆在合法範圍內。
    /// 建議在系統初始化或載入外部 ROM 時呼叫此方法。
    pub fn validate_rom(&self) -> bool {
        let mut pos = 0;
        let len = self.rom_matrix.len();
        if len == 0 {
            return true;
        }
        while pos < len {
            if pos + 1 > len { return false; }
            let flags = self.rom_matrix[pos];
            pos += 1;
            
            if flags & 1 != 0 { pos += 1; }
            if flags & 2 != 0 { pos += 2; }
            
            if pos + 2 > len { return false; }
            let fail_offset = u16::from_le_bytes([self.rom_matrix[pos], self.rom_matrix[pos + 1]]) as usize;
            if fail_offset >= len { return false; }
            pos += 2;
            
            if pos + 1 > len { return false; }
            let num_transitions = self.rom_matrix[pos] as usize;
            pos += 1;
            
            if pos + num_transitions * 3 > len { return false; }
            for _ in 0..num_transitions {
                let child_offset = u16::from_le_bytes([self.rom_matrix[pos + 1], self.rom_matrix[pos + 2]]) as usize;
                if child_offset >= len { return false; }
                pos += 3;
            }
        }
        pos == len
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

/// `UnionCode` 核心引擎。結合了 O(N) FST 靜態路由解析，
/// 負責將人類語言字串映射為極限壓縮的 3-Bytes `CompressedIntent`。
///
/// # Examples
/// ```
/// use union_code::{UnionCode, CompressedIntent};
/// 
/// // 建立 UnionCode 引擎
/// let uc = UnionCode::default();
/// 
/// // 解碼輸入（完全無記憶體分配、Lock-free、Thread-safe）
/// let result = uc.decode("請幫我拿咖啡".as_bytes());
/// // assert_eq!(result, Ok(CompressedIntent { opcode: 0x20, payload_id: 0x0A42 }));
/// ```
#[derive(Default)]
pub struct UnionCode {
    pub fst: FstEngine,
}

impl UnionCode {
    pub fn new(fst: FstEngine) -> Self {
        Self { fst }
    }

    /// 核心轉譯管線：人類語言 -> 3 Bytes 二進位指令
    pub fn decode(&self, human_input: &[u8]) -> Result<CompressedIntent, u8> {
        // FST 靜態路由解析 (O(N), ~幾十奈秒)
        if let Some(intent) = self.fst.parse_stream(human_input) {
            return Ok(intent);
        }

        // 未知意圖降級 (Fallback)
        // 觸發 0x06 NotFound，交由雲端 LLM 非同步解析，或引導用戶重新輸入
        Err(0x06) // OpCode::NotFound
    }
}


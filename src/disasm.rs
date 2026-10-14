//! Desassemblador 65C816 com rastreamento dos flags M/X (tamanho dos operandos imediatos).
//!
//! Limitação: é uma varredura linear. Ele acompanha `REP`/`SEP`, mas não segue desvios,
//! então depois de um salto o estado de M/X pode estar errado. Use `--m16` / `--x16`
//! para corrigir o estado inicial quando o trecho começar em modo de 16 bits.

use crate::optable::{Mode, OPS};
use crate::rom::Rom;
use crate::wram;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Flags {
    /// true = acumulador de 8 bits (M=1)
    pub m8: bool,
    /// true = registradores de índice de 8 bits (X=1)
    pub x8: bool,
}

impl Default for Flags {
    /// Estado após o RESET: modo de 8 bits.
    fn default() -> Self {
        Flags { m8: true, x8: true }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Insn {
    pub bank: u8,
    pub addr: u16,
    pub bytes: Vec<u8>,
    pub op: u8,
    pub mode: Mode,
    /// Operando bruto (little-endian), sem sinal.
    pub value: u32,
    pub mnemonic: &'static str,
    pub operand: String,
    pub comment: Option<String>,
}

impl Insn {
    pub fn text(&self) -> String {
        if self.operand.is_empty() {
            self.mnemonic.to_string()
        } else {
            format!("{:<4} {}", self.mnemonic, self.operand)
        }
    }
    pub fn ends_function(&self) -> bool {
        matches!(self.mnemonic, "RTS" | "RTL" | "RTI")
    }
}

fn operand_len(mode: Mode, f: Flags) -> usize {
    use Mode::*;
    match mode {
        Imp | Acc => 0,
        ImmM => {
            if f.m8 {
                1
            } else {
                2
            }
        }
        ImmX => {
            if f.x8 {
                1
            } else {
                2
            }
        }
        Imm8 | Dp | DpX | DpY | DpInd | DpIndX | DpIndY | DpIndLong | DpIndLongY | Stack
        | StackIndY | Rel8 => 1,
        Abs | AbsX | AbsY | AbsInd | AbsIndX | AbsIndLong | Rel16 | BlockMove => 2,
        Long | LongX => 3,
    }
}

/// Decodifica uma instrução em `bank:addr`. `dp0` anota acessos de direct page assumindo D = 0.
pub fn decode(rom: &Rom, bank: u8, addr: u16, f: Flags, dp0: bool) -> Option<Insn> {
    let op = rom.read(bank, addr)?;
    let (mn, mode) = OPS[op as usize];
    let n = operand_len(mode, f);
    let mut bytes = vec![op];
    for i in 1..=n {
        bytes.push(rom.read(bank, addr.wrapping_add(i as u16))?);
    }
    let v = bytes[1..]
        .iter()
        .rev()
        .fold(0u32, |a, &b| (a << 8) | b as u32);
    let next = addr.wrapping_add(n as u16 + 1);
    let w8 = format!("{:02X}", v);
    let w16 = format!("{:04X}", v);
    let w24 = format!("{:06X}", v);

    use Mode::*;
    let mut annot: Option<u32> = None;
    let operand = match mode {
        Imp => String::new(),
        Acc => "A".to_string(),
        ImmM => {
            if f.m8 {
                format!("#${w8}")
            } else {
                format!("#${w16}")
            }
        }
        ImmX => {
            if f.x8 {
                format!("#${w8}")
            } else {
                format!("#${w16}")
            }
        }
        Imm8 => format!("#${w8}"),
        Dp => {
            if dp0 {
                annot = Some(v)
            }
            format!("${w8}")
        }
        DpX => {
            if dp0 {
                annot = Some(v)
            }
            format!("${w8},X")
        }
        DpY => {
            if dp0 {
                annot = Some(v)
            }
            format!("${w8},Y")
        }
        DpInd => format!("(${w8})"),
        DpIndX => format!("(${w8},X)"),
        DpIndY => format!("(${w8}),Y"),
        DpIndLong => format!("[${w8}]"),
        DpIndLongY => format!("[${w8}],Y"),
        Stack => format!("${w8},S"),
        StackIndY => format!("(${w8},S),Y"),
        Abs => {
            if matches!(mn, "JSR" | "JMP") {
                format!("${w16}")
            } else {
                annot = Some(v);
                format!("${w16}")
            }
        }
        AbsX => {
            annot = Some(v);
            format!("${w16},X")
        }
        AbsY => {
            annot = Some(v);
            format!("${w16},Y")
        }
        AbsInd => format!("(${w16})"),
        AbsIndX => format!("(${w16},X)"),
        AbsIndLong => format!("[${w16}]"),
        Long => {
            if (v >> 16) == 0x7E || (v >> 16) == 0x7F || (v >> 16) == 0 {
                annot = Some(v & 0xFFFF)
            }
            format!("${w24}")
        }
        LongX => format!("${w24},X"),
        Rel8 => format!("${:04X}", next.wrapping_add(v as u8 as i8 as i16 as u16)),
        Rel16 => format!("${:04X}", next.wrapping_add(v as u16)),
        BlockMove => format!("${:02X},${:02X}", bytes[2], bytes[1]), // origem,destino
    };
    let comment = annot.and_then(wram::name);
    Some(Insn {
        bank,
        addr,
        bytes,
        op,
        mode,
        value: v,
        mnemonic: mn,
        operand,
        comment,
    })
}

/// Atualiza M/X depois de uma instrução `REP`/`SEP`.
pub fn step_flags(f: &mut Flags, insn: &Insn) {
    if insn.bytes.len() == 2 {
        let v = insn.bytes[1];
        match insn.mnemonic {
            "REP" => {
                if v & 0x20 != 0 {
                    f.m8 = false
                }
                if v & 0x10 != 0 {
                    f.x8 = false
                }
            }
            "SEP" => {
                if v & 0x20 != 0 {
                    f.m8 = true
                }
                if v & 0x10 != 0 {
                    f.x8 = true
                }
            }
            _ => {}
        }
    }
}

/// Desmonta até `count` instruções (ou até o fim da função, se `stop_at_return`).
pub fn disassemble(
    rom: &Rom,
    bank: u8,
    start: u16,
    count: usize,
    mut f: Flags,
    dp0: bool,
    stop_at_return: bool,
) -> Vec<(Insn, Flags)> {
    let mut out = Vec::new();
    let mut a = start;
    for _ in 0..count {
        let Some(i) = decode(rom, bank, a, f, dp0) else {
            break;
        };
        let before = f;
        step_flags(&mut f, &i);
        a = a.wrapping_add(i.bytes.len() as u16);
        let end = stop_at_return && i.ends_function();
        out.push((i, before));
        if end {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom_with(bytes: &[u8]) -> Rom {
        let mut d = vec![0u8; 0x8000];
        d[..bytes.len()].copy_from_slice(bytes);
        Rom::from_bytes(d)
    }

    #[test]
    fn reset_sequence() {
        // Início real do RESET do Final Fight: CLC / XCE / LDA #$00 / STA $7EFFF9
        let rom = rom_with(&[0x18, 0xFB, 0xA9, 0x00, 0x8F, 0xF9, 0xFF, 0x7E]);
        let r = disassemble(&rom, 0, 0x8000, 4, Flags::default(), false, false);
        let t: Vec<String> = r.iter().map(|(i, _)| i.text()).collect();
        assert_eq!(t, ["CLC", "XCE", "LDA  #$00", "STA  $7EFFF9"]);
    }

    #[test]
    fn rep_changes_operand_size() {
        let rom = rom_with(&[0xC2, 0x10, 0xA2, 0xF7, 0x1F, 0x9A]);
        let r = disassemble(&rom, 0, 0x8000, 3, Flags::default(), false, false);
        assert_eq!(r[1].0.text(), "LDX  #$1FF7");
        assert_eq!(r[2].0.text(), "TXS");
    }

    #[test]
    fn branch_and_annotation() {
        // LDA $00C9 / BNE -4
        let rom = rom_with(&[0xAD, 0xC9, 0x00, 0xD0, 0xFB]);
        let r = disassemble(&rom, 0, 0x8000, 2, Flags::default(), false, false);
        assert_eq!(r[0].0.comment.as_deref(), Some("frame_counter"));
        assert_eq!(r[1].0.operand, "$8000");
    }
}

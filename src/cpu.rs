//! CPU WDC 65C816 (a do SNES): interpretador completo dos 256 opcodes.
//!
//! É a *referência semântica* da recompilação: o código gerado depois tem que produzir
//! exatamente o mesmo estado que este interpretador.
//!
//! Temporização aproximada: cada acesso ao barramento custa 6/8/12 ciclos mestres e cada
//! instrução soma alguns ciclos internos. Serve para NMI/IRQ por scanline, não para
//! efeitos que dependem de ciclos exatos.

use crate::bus::Bus;
use crate::optable::{Mode, OPS};
use std::sync::OnceLock;

pub const N: u8 = 0x80;
pub const V: u8 = 0x40;
pub const M: u8 = 0x20;
pub const X: u8 = 0x10;
pub const D: u8 = 0x08;
pub const I: u8 = 0x04;
pub const Z: u8 = 0x02;
pub const C: u8 = 0x01;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Adc,
    And,
    Asl,
    Bcc,
    Bcs,
    Beq,
    Bit,
    Bmi,
    Bne,
    Bpl,
    Bra,
    Brk,
    Brl,
    Bvc,
    Bvs,
    Clc,
    Cld,
    Cli,
    Clv,
    Cmp,
    Cop,
    Cpx,
    Cpy,
    Dec,
    Dex,
    Dey,
    Eor,
    Inc,
    Inx,
    Iny,
    Jml,
    Jmp,
    Jsl,
    Jsr,
    Lda,
    Ldx,
    Ldy,
    Lsr,
    Mvn,
    Mvp,
    Nop,
    Ora,
    Pea,
    Pei,
    Per,
    Pha,
    Phb,
    Phd,
    Phk,
    Php,
    Phx,
    Phy,
    Pla,
    Plb,
    Pld,
    Plp,
    Plx,
    Ply,
    Rep,
    Rol,
    Ror,
    Rti,
    Rtl,
    Rts,
    Sbc,
    Sec,
    Sed,
    Sei,
    Sep,
    Sta,
    Stp,
    Stx,
    Sty,
    Stz,
    Tax,
    Tay,
    Tcd,
    Tcs,
    Tdc,
    Trb,
    Tsb,
    Tsc,
    Tsx,
    Txa,
    Txs,
    Txy,
    Tya,
    Tyx,
    Wai,
    Wdm,
    Xba,
    Xce,
}

fn op_from(s: &str) -> Op {
    use Op::*;
    match s {
        "ADC" => Adc,
        "AND" => And,
        "ASL" => Asl,
        "BCC" => Bcc,
        "BCS" => Bcs,
        "BEQ" => Beq,
        "BIT" => Bit,
        "BMI" => Bmi,
        "BNE" => Bne,
        "BPL" => Bpl,
        "BRA" => Bra,
        "BRK" => Brk,
        "BRL" => Brl,
        "BVC" => Bvc,
        "BVS" => Bvs,
        "CLC" => Clc,
        "CLD" => Cld,
        "CLI" => Cli,
        "CLV" => Clv,
        "CMP" => Cmp,
        "COP" => Cop,
        "CPX" => Cpx,
        "CPY" => Cpy,
        "DEC" => Dec,
        "DEX" => Dex,
        "DEY" => Dey,
        "EOR" => Eor,
        "INC" => Inc,
        "INX" => Inx,
        "INY" => Iny,
        "JML" => Jml,
        "JMP" => Jmp,
        "JSL" => Jsl,
        "JSR" => Jsr,
        "LDA" => Lda,
        "LDX" => Ldx,
        "LDY" => Ldy,
        "LSR" => Lsr,
        "MVN" => Mvn,
        "MVP" => Mvp,
        "NOP" => Nop,
        "ORA" => Ora,
        "PEA" => Pea,
        "PEI" => Pei,
        "PER" => Per,
        "PHA" => Pha,
        "PHB" => Phb,
        "PHD" => Phd,
        "PHK" => Phk,
        "PHP" => Php,
        "PHX" => Phx,
        "PHY" => Phy,
        "PLA" => Pla,
        "PLB" => Plb,
        "PLD" => Pld,
        "PLP" => Plp,
        "PLX" => Plx,
        "PLY" => Ply,
        "REP" => Rep,
        "ROL" => Rol,
        "ROR" => Ror,
        "RTI" => Rti,
        "RTL" => Rtl,
        "RTS" => Rts,
        "SBC" => Sbc,
        "SEC" => Sec,
        "SED" => Sed,
        "SEI" => Sei,
        "SEP" => Sep,
        "STA" => Sta,
        "STP" => Stp,
        "STX" => Stx,
        "STY" => Sty,
        "STZ" => Stz,
        "TAX" => Tax,
        "TAY" => Tay,
        "TCD" => Tcd,
        "TCS" => Tcs,
        "TDC" => Tdc,
        "TRB" => Trb,
        "TSB" => Tsb,
        "TSC" => Tsc,
        "TSX" => Tsx,
        "TXA" => Txa,
        "TXS" => Txs,
        "TXY" => Txy,
        "TYA" => Tya,
        "TYX" => Tyx,
        "WAI" => Wai,
        "WDM" => Wdm,
        "XBA" => Xba,
        "XCE" => Xce,
        other => panic!("mnemônico desconhecido: {other}"),
    }
}

fn table() -> &'static [(Op, Mode); 256] {
    static T: OnceLock<[(Op, Mode); 256]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [(Op::Nop, Mode::Imp); 256];
        for (i, (mn, mode)) in OPS.iter().enumerate() {
            t[i] = (op_from(mn), *mode);
        }
        t
    })
}

#[derive(Clone, Debug)]
pub struct Cpu {
    pub a: u16,
    pub x: u16,
    pub y: u16,
    pub s: u16,
    pub d: u16,
    pub db: u8,
    pub pb: u8,
    pub pc: u16,
    pub p: u8,
    pub e: bool,
    pub waiting: bool,
    pub stopped: bool,
    pub instr_count: u64,
    pub decimal_ops: u64,
}

impl Default for Cpu {
    fn default() -> Self {
        Cpu::new()
    }
}

impl Cpu {
    pub fn new() -> Cpu {
        Cpu {
            a: 0,
            x: 0,
            y: 0,
            s: 0x01FF,
            d: 0,
            db: 0,
            pb: 0,
            pc: 0,
            p: 0x34,
            e: true,
            waiting: false,
            stopped: false,
            instr_count: 0,
            decimal_ops: 0,
        }
    }

    pub fn reset(&mut self, bus: &mut Bus) {
        self.e = true;
        self.p = 0x34;
        self.s = 0x01FF;
        self.d = 0;
        self.db = 0;
        self.pb = 0;
        self.x &= 0xFF;
        self.y &= 0xFF;
        self.waiting = false;
        self.stopped = false;
        self.pc = self.vector(bus, 0xFFFC);
    }

    // ---------- utilidades de acesso ----------
    #[inline]
    fn m8(&self) -> bool {
        self.p & M != 0
    }
    #[inline]
    fn x8(&self) -> bool {
        self.p & X != 0
    }
    #[inline]
    fn flag(&self, f: u8) -> bool {
        self.p & f != 0
    }
    #[inline]
    fn set_flag(&mut self, f: u8, on: bool) {
        if on {
            self.p |= f
        } else {
            self.p &= !f
        }
    }

    fn vector(&mut self, bus: &mut Bus, addr: u16) -> u16 {
        let lo = bus.read(addr as u32) as u16;
        lo | (bus.read(addr as u32 + 1) as u16) << 8
    }

    #[inline]
    fn fetch8(&mut self, bus: &mut Bus) -> u8 {
        let v = bus.read(((self.pb as u32) << 16) | self.pc as u32);
        self.pc = self.pc.wrapping_add(1);
        v
    }
    fn fetch16(&mut self, bus: &mut Bus) -> u16 {
        let lo = self.fetch8(bus) as u16;
        lo | (self.fetch8(bus) as u16) << 8
    }
    fn fetch24(&mut self, bus: &mut Bus) -> u32 {
        let lo = self.fetch16(bus) as u32;
        lo | (self.fetch8(bus) as u32) << 16
    }

    /// Lê 16 bits. `wrap16` = o segundo byte dá a volta dentro do banco (direct page/pilha).
    fn rd16(&mut self, bus: &mut Bus, addr: u32, wrap16: bool) -> u16 {
        let lo = bus.read(addr & 0xFF_FFFF) as u16;
        let hi_addr = if wrap16 {
            (addr & 0xFF_0000) | (addr.wrapping_add(1) & 0xFFFF)
        } else {
            addr.wrapping_add(1) & 0xFF_FFFF
        };
        lo | (bus.read(hi_addr) as u16) << 8
    }
    fn wr16(&mut self, bus: &mut Bus, addr: u32, v: u16, wrap16: bool) {
        bus.write(addr & 0xFF_FFFF, v as u8);
        let hi_addr = if wrap16 {
            (addr & 0xFF_0000) | (addr.wrapping_add(1) & 0xFFFF)
        } else {
            addr.wrapping_add(1) & 0xFF_FFFF
        };
        bus.write(hi_addr, (v >> 8) as u8);
    }

    // ---------- pilha ----------
    fn push8(&mut self, bus: &mut Bus, v: u8) {
        bus.write(self.s as u32, v);
        self.s = self.s.wrapping_sub(1);
    }
    fn push16(&mut self, bus: &mut Bus, v: u16) {
        self.push8(bus, (v >> 8) as u8);
        self.push8(bus, v as u8);
    }
    fn push_interrupt8(&mut self, bus: &mut Bus, v: u8) {
        if self.e {
            self.s = 0x0100 | (self.s & 0xFF);
        }
        bus.write(self.s as u32, v);
        self.s = self.s.wrapping_sub(1);
        if self.e {
            self.s = 0x0100 | (self.s & 0xFF);
        }
    }
    fn push_interrupt16(&mut self, bus: &mut Bus, v: u16) {
        self.push_interrupt8(bus, (v >> 8) as u8);
        self.push_interrupt8(bus, v as u8);
    }
    fn pull8(&mut self, bus: &mut Bus) -> u8 {
        self.s = self.s.wrapping_add(1);
        bus.read(self.s as u32)
    }
    fn pull16(&mut self, bus: &mut Bus) -> u16 {
        let lo = self.pull8(bus) as u16;
        lo | (self.pull8(bus) as u16) << 8
    }

    // ---------- flags ----------
    fn nz(&mut self, v: u16, wide: bool) {
        let (n, z) = if wide {
            (v & 0x8000 != 0, v == 0)
        } else {
            (v & 0x80 != 0, v & 0xFF == 0)
        };
        self.set_flag(N, n);
        self.set_flag(Z, z);
    }
    fn set_p(&mut self, v: u8) {
        self.p = v;
        if self.e {
            self.p |= 0x30
        }
        if self.x8() {
            self.x &= 0xFF;
            self.y &= 0xFF
        }
    }
    #[inline]
    fn a_val(&self, wide: bool) -> u16 {
        if wide {
            self.a
        } else {
            self.a & 0xFF
        }
    }
    #[inline]
    fn set_a(&mut self, v: u16, wide: bool) {
        if wide {
            self.a = v
        } else {
            self.a = (self.a & 0xFF00) | (v & 0xFF)
        }
    }

    // ---------- endereçamento ----------
    #[inline]
    fn idx_x(&self) -> u16 {
        if self.x8() {
            self.x & 0xFF
        } else {
            self.x
        }
    }
    #[inline]
    fn idx_y(&self) -> u16 {
        if self.x8() {
            self.y & 0xFF
        } else {
            self.y
        }
    }

    /// Endereço efetivo de 24 bits e se o acesso de 16 bits dá a volta no banco.
    fn ea(&mut self, bus: &mut Bus, mode: Mode) -> (u32, bool) {
        use Mode::*;
        let db = (self.db as u32) << 16;
        match mode {
            Dp => {
                let o = self.fetch8(bus) as u16;
                (self.d.wrapping_add(o) as u32, true)
            }
            DpX => {
                let o = self.fetch8(bus) as u16;
                (
                    self.d.wrapping_add(o).wrapping_add(self.idx_x()) as u32,
                    true,
                )
            }
            DpY => {
                let o = self.fetch8(bus) as u16;
                (
                    self.d.wrapping_add(o).wrapping_add(self.idx_y()) as u32,
                    true,
                )
            }
            DpInd => {
                let o = self.fetch8(bus) as u16;
                let ptr = self.rd16(bus, self.d.wrapping_add(o) as u32, true) as u32;
                (db | ptr, false)
            }
            DpIndX => {
                let o = self.fetch8(bus) as u16;
                let p = self.d.wrapping_add(o).wrapping_add(self.idx_x()) as u32;
                let ptr = self.rd16(bus, p, true) as u32;
                (db | ptr, false)
            }
            DpIndY => {
                let o = self.fetch8(bus) as u16;
                let ptr = self.rd16(bus, self.d.wrapping_add(o) as u32, true) as u32;
                (
                    (db | ptr).wrapping_add(self.idx_y() as u32) & 0xFF_FFFF,
                    false,
                )
            }
            DpIndLong | DpIndLongY => {
                let o = self.fetch8(bus) as u16;
                let p = self.d.wrapping_add(o);
                let lo = self.rd16(bus, p as u32, true) as u32;
                let bank = bus.read(p.wrapping_add(2) as u32) as u32;
                let base = (bank << 16) | lo;
                let a = if mode == DpIndLongY {
                    base.wrapping_add(self.idx_y() as u32) & 0xFF_FFFF
                } else {
                    base
                };
                (a, false)
            }
            Stack => {
                let o = self.fetch8(bus) as u16;
                (self.s.wrapping_add(o) as u32, true)
            }
            StackIndY => {
                let o = self.fetch8(bus) as u16;
                let ptr = self.rd16(bus, self.s.wrapping_add(o) as u32, true) as u32;
                (
                    (db | ptr).wrapping_add(self.idx_y() as u32) & 0xFF_FFFF,
                    false,
                )
            }
            Abs => {
                let a = self.fetch16(bus) as u32;
                (db | a, false)
            }
            AbsX => {
                let a = self.fetch16(bus) as u32;
                (
                    (db | a).wrapping_add(self.idx_x() as u32) & 0xFF_FFFF,
                    false,
                )
            }
            AbsY => {
                let a = self.fetch16(bus) as u32;
                (
                    (db | a).wrapping_add(self.idx_y() as u32) & 0xFF_FFFF,
                    false,
                )
            }
            Long => (self.fetch24(bus), false),
            LongX => {
                let a = self.fetch24(bus);
                (a.wrapping_add(self.idx_x() as u32) & 0xFF_FFFF, false)
            }
            other => panic!("modo sem endereço efetivo: {other:?}"),
        }
    }

    fn read_operand(&mut self, bus: &mut Bus, mode: Mode, wide: bool) -> u16 {
        match mode {
            Mode::ImmM | Mode::ImmX => {
                if wide {
                    self.fetch16(bus)
                } else {
                    self.fetch8(bus) as u16
                }
            }
            _ => {
                let (a, w) = self.ea(bus, mode);
                if wide {
                    self.rd16(bus, a, w)
                } else {
                    bus.read(a) as u16
                }
            }
        }
    }

    // ---------- aritmética ----------
    fn adc(&mut self, v: u16, wide: bool) {
        let (mask, msb, digits) = if wide {
            (0xFFFFu32, 0x8000u32, 4)
        } else {
            (0xFFu32, 0x80u32, 2)
        };
        let a = self.a_val(wide) as u32;
        let v = v as u32;
        let c = (self.p & C) as u32;
        let (res, carry, overflow_result);
        if self.flag(D) {
            self.decimal_ops += 1;
            let (mut r, mut raw, mut cy) = (0u32, 0u32, c);
            for i in 0..digits {
                let mut s = ((a >> (4 * i)) & 0xF) + ((v >> (4 * i)) & 0xF) + cy;
                raw |= (s & 0xF) << (4 * i);
                if s > 9 {
                    s += 6;
                    cy = 1
                } else {
                    cy = 0
                }
                r |= (s & 0xF) << (4 * i);
            }
            res = r;
            carry = cy != 0;
            overflow_result = raw;
        } else {
            let sum = a + v + c;
            res = sum & mask;
            carry = sum > mask;
            overflow_result = res;
        }
        self.set_flag(V, (a ^ overflow_result) & (v ^ overflow_result) & msb != 0);
        self.set_flag(C, carry);
        self.set_a(res as u16, wide);
        self.nz(res as u16, wide);
    }

    fn sbc(&mut self, v: u16, wide: bool) {
        if !self.flag(D) {
            let mask = if wide { 0xFFFF } else { 0xFF };
            self.adc(!v & mask, wide);
            return;
        }
        self.decimal_ops += 1;
        let (msb, digits) = if wide { (0x8000u32, 4) } else { (0x80u32, 2) };
        let a = self.a_val(wide) as u32;
        let vv = v as u32;
        let (mut r, mut raw, mut borrow) = (0u32, 0u32, 1 - (self.p & C) as i32);
        for i in 0..digits {
            let mut d = ((a >> (4 * i)) & 0xF) as i32 - ((vv >> (4 * i)) & 0xF) as i32 - borrow;
            raw |= ((d as u32) & 0xF) << (4 * i);
            if d < 0 {
                d += 10;
                borrow = 1
            } else {
                borrow = 0
            }
            r |= ((d as u32) & 0xF) << (4 * i);
        }
        self.set_flag(V, (a ^ raw) & (a ^ vv) & msb != 0);
        self.set_flag(C, borrow == 0);
        self.set_a(r as u16, wide);
        self.nz(r as u16, wide);
    }

    fn cmp(&mut self, reg: u16, v: u16, wide: bool) {
        let mask: u16 = if wide { 0xFFFF } else { 0xFF };
        let (reg, v) = (reg & mask, v & mask);
        self.set_flag(C, reg >= v);
        self.nz(reg.wrapping_sub(v) & mask, wide);
    }

    fn rmw_op(&mut self, o: Op, v: u16, wide: bool) -> u16 {
        let (msb, mask) = if wide {
            (0x8000u16, 0xFFFFu16)
        } else {
            (0x80u16, 0xFFu16)
        };
        let v = v & mask;
        let r = match o {
            Op::Asl => {
                self.set_flag(C, v & msb != 0);
                (v << 1) & mask
            }
            Op::Lsr => {
                self.set_flag(C, v & 1 != 0);
                v >> 1
            }
            Op::Rol => {
                let c = self.flag(C) as u16;
                self.set_flag(C, v & msb != 0);
                ((v << 1) | c) & mask
            }
            Op::Ror => {
                let c = self.flag(C);
                self.set_flag(C, v & 1 != 0);
                (v >> 1) | if c { msb } else { 0 }
            }
            Op::Inc => v.wrapping_add(1) & mask,
            Op::Dec => v.wrapping_sub(1) & mask,
            Op::Tsb => {
                let a = self.a_val(wide);
                self.set_flag(Z, a & v == 0);
                v | a
            }
            Op::Trb => {
                let a = self.a_val(wide);
                self.set_flag(Z, a & v == 0);
                v & !a
            }
            _ => unreachable!(),
        };
        if !matches!(o, Op::Tsb | Op::Trb) {
            self.nz(r, wide)
        }
        r
    }

    // ---------- interrupções ----------
    pub fn interrupt(&mut self, bus: &mut Bus, native: u16, emu: u16, brk: bool) {
        if self.e {
            self.s = 0x0100 | (self.s & 0xFF);
        }
        if self.e {
            self.push_interrupt16(bus, self.pc);
            self.push_interrupt8(
                bus,
                if brk {
                    self.p | 0x30
                } else {
                    (self.p | 0x20) & !0x10
                },
            );
        } else {
            self.push_interrupt8(bus, self.pb);
            self.push16(bus, self.pc);
            self.push_interrupt8(bus, self.p);
        }
        self.p |= I;
        self.p &= !D;
        self.pb = 0;
        self.pc = self.vector(bus, if self.e { emu } else { native });
        if self.e {
            self.s = 0x0100 | (self.s & 0xFF);
        }
        bus.cycles += 6 * 2;
    }
    pub fn nmi(&mut self, bus: &mut Bus) {
        self.interrupt(bus, 0xFFEA, 0xFFFA, false)
    }
    pub fn irq(&mut self, bus: &mut Bus) {
        self.interrupt(bus, 0xFFEE, 0xFFFE, false)
    }

    // ---------- execução ----------
    /// Executa uma instrução e devolve os ciclos mestres consumidos.
    pub fn step(&mut self, bus: &mut Bus) -> u32 {
        let start = bus.cycles;
        if self.stopped || self.waiting {
            bus.cycles += 6;
            return 6;
        }
        if self.e {
            self.s = 0x0100 | (self.s & 0xFF);
        }
        let opc = self.fetch8(bus);
        let (o, mode) = table()[opc as usize];
        self.instr_count += 1;
        let mut int: u64 = 1;
        use Op::*;

        match o {
            Lda | Ldx | Ldy | And | Ora | Eor | Adc | Sbc | Cmp | Cpx | Cpy | Bit => {
                let idx = matches!(o, Ldx | Ldy | Cpx | Cpy);
                let wide = if idx { !self.x8() } else { !self.m8() };
                let v = self.read_operand(bus, mode, wide);
                match o {
                    Lda => {
                        self.set_a(v, wide);
                        self.nz(v, wide)
                    }
                    Ldx => {
                        self.x = v;
                        self.nz(v, wide)
                    }
                    Ldy => {
                        self.y = v;
                        self.nz(v, wide)
                    }
                    And => {
                        let r = self.a_val(wide) & v;
                        self.set_a(r, wide);
                        self.nz(r, wide)
                    }
                    Ora => {
                        let r = self.a_val(wide) | v;
                        self.set_a(r, wide);
                        self.nz(r, wide)
                    }
                    Eor => {
                        let r = self.a_val(wide) ^ v;
                        self.set_a(r, wide);
                        self.nz(r, wide)
                    }
                    Adc => self.adc(v, wide),
                    Sbc => self.sbc(v, wide),
                    Cmp => {
                        let r = self.a_val(wide);
                        self.cmp(r, v, wide)
                    }
                    Cpx => {
                        let r = self.x;
                        self.cmp(r, v, wide)
                    }
                    Cpy => {
                        let r = self.y;
                        self.cmp(r, v, wide)
                    }
                    Bit => {
                        let (msb, mask) = if wide {
                            (0x8000u16, 0xFFFFu16)
                        } else {
                            (0x80, 0xFF)
                        };
                        self.set_flag(Z, self.a_val(wide) & v & mask == 0);
                        if mode != Mode::ImmM {
                            self.set_flag(N, v & msb != 0);
                            self.set_flag(V, v & (msb >> 1) != 0);
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Sta | Stz | Stx | Sty => {
                let wide = if matches!(o, Stx | Sty) {
                    !self.x8()
                } else {
                    !self.m8()
                };
                let v = match o {
                    Sta => self.a,
                    Stx => self.x,
                    Sty => self.y,
                    _ => 0,
                };
                let (a, w) = self.ea(bus, mode);
                if wide {
                    self.wr16(bus, a, v, w)
                } else {
                    bus.write(a, v as u8)
                }
            }
            Asl | Lsr | Rol | Ror | Inc | Dec | Tsb | Trb => {
                let wide = !self.m8();
                if mode == Mode::Acc {
                    let v = self.a_val(wide);
                    let r = self.rmw_op(o, v, wide);
                    self.set_a(r, wide);
                } else {
                    let (a, w) = self.ea(bus, mode);
                    let v = if wide {
                        self.rd16(bus, a, w)
                    } else {
                        bus.read(a) as u16
                    };
                    let r = self.rmw_op(o, v, wide);
                    if wide {
                        self.wr16(bus, a, r, w)
                    } else {
                        bus.write(a, r as u8)
                    }
                    int += 1;
                }
            }
            Inx | Iny | Dex | Dey => {
                let wide = !self.x8();
                let mask: u16 = if wide { 0xFFFF } else { 0xFF };
                let r = match o {
                    Inx => self.x.wrapping_add(1),
                    Iny => self.y.wrapping_add(1),
                    Dex => self.x.wrapping_sub(1),
                    _ => self.y.wrapping_sub(1),
                } & mask;
                if matches!(o, Inx | Dex) {
                    self.x = r
                } else {
                    self.y = r
                }
                self.nz(r, wide);
            }
            Bcc | Bcs | Beq | Bne | Bmi | Bpl | Bvc | Bvs | Bra => {
                let off = self.fetch8(bus) as i8;
                let take = match o {
                    Bcc => !self.flag(C),
                    Bcs => self.flag(C),
                    Beq => self.flag(Z),
                    Bne => !self.flag(Z),
                    Bmi => self.flag(N),
                    Bpl => !self.flag(N),
                    Bvc => !self.flag(V),
                    Bvs => self.flag(V),
                    _ => true,
                };
                if take {
                    self.pc = self.pc.wrapping_add(off as i16 as u16);
                    int += 1
                }
            }
            Brl => {
                let off = self.fetch16(bus);
                self.pc = self.pc.wrapping_add(off);
                int += 1
            }
            Jmp => match mode {
                Mode::Abs => self.pc = self.fetch16(bus),
                Mode::AbsInd => {
                    let p = self.fetch16(bus) as u32;
                    self.pc = self.rd16(bus, p, true);
                }
                _ => {
                    // (abs,X): lê no banco do programa
                    let p = self.fetch16(bus).wrapping_add(self.idx_x());
                    self.pc = self.rd16(bus, ((self.pb as u32) << 16) | p as u32, true);
                }
            },
            Jml => match mode {
                Mode::Long => {
                    let a = self.fetch24(bus);
                    self.pc = a as u16;
                    self.pb = (a >> 16) as u8
                }
                _ => {
                    // [abs]
                    let p = self.fetch16(bus);
                    self.pc = self.rd16(bus, p as u32, true);
                    self.pb = bus.read(p.wrapping_add(2) as u32);
                }
            },
            Jsr => {
                if mode == Mode::Abs {
                    let t = self.fetch16(bus);
                    let ret = self.pc.wrapping_sub(1);
                    self.push16(bus, ret);
                    self.pc = t;
                } else {
                    // (abs,X)
                    let lo = self.fetch8(bus) as u16;
                    let ret = self.pc;
                    self.push16(bus, ret);
                    let hi = self.fetch8(bus) as u16;
                    let p = (lo | hi << 8).wrapping_add(self.idx_x());
                    self.pc = self.rd16(bus, ((self.pb as u32) << 16) | p as u32, true);
                }
                int += 1;
            }
            Jsl => {
                let a = self.fetch24(bus);
                self.push8(bus, self.pb);
                let ret = self.pc.wrapping_sub(1);
                self.push16(bus, ret);
                self.pc = a as u16;
                self.pb = (a >> 16) as u8;
                int += 1;
            }
            Rts => {
                self.pc = self.pull16(bus).wrapping_add(1);
                int += 2
            }
            Rtl => {
                self.pc = self.pull16(bus).wrapping_add(1);
                self.pb = self.pull8(bus);
                int += 1
            }
            Rti => {
                let p = self.pull8(bus);
                self.set_p(p);
                self.pc = self.pull16(bus);
                if !self.e {
                    self.pb = self.pull8(bus)
                }
                int += 1;
            }
            Brk | Cop => {
                let _sig = self.fetch8(bus);
                if o == Brk {
                    self.interrupt(bus, 0xFFE6, 0xFFFE, true)
                } else {
                    self.interrupt(bus, 0xFFE4, 0xFFF4, true)
                }
            }
            Pea => {
                let v = self.fetch16(bus);
                self.push16(bus, v)
            }
            Pei => {
                let o8 = self.fetch8(bus) as u16;
                let v = self.rd16(bus, self.d.wrapping_add(o8) as u32, true);
                self.push16(bus, v);
            }
            Per => {
                let off = self.fetch16(bus);
                let v = self.pc.wrapping_add(off);
                self.push16(bus, v)
            }
            Pha => {
                if self.m8() {
                    let v = self.a as u8;
                    self.push8(bus, v)
                } else {
                    let v = self.a;
                    self.push16(bus, v)
                }
            }
            Phx => {
                if self.x8() {
                    let v = self.x as u8;
                    self.push8(bus, v)
                } else {
                    let v = self.x;
                    self.push16(bus, v)
                }
            }
            Phy => {
                if self.x8() {
                    let v = self.y as u8;
                    self.push8(bus, v)
                } else {
                    let v = self.y;
                    self.push16(bus, v)
                }
            }
            Phb => {
                let v = self.db;
                self.push8(bus, v)
            }
            Phk => {
                let v = self.pb;
                self.push8(bus, v)
            }
            Phd => {
                let v = self.d;
                self.push16(bus, v)
            }
            Php => {
                let v = self.p;
                self.push8(bus, v)
            }
            Pla => {
                let wide = !self.m8();
                let v = if wide {
                    self.pull16(bus)
                } else {
                    self.pull8(bus) as u16
                };
                self.set_a(v, wide);
                self.nz(v, wide);
                int += 1;
            }
            Plx | Ply => {
                let wide = !self.x8();
                let v = if wide {
                    self.pull16(bus)
                } else {
                    self.pull8(bus) as u16
                };
                if o == Plx {
                    self.x = v
                } else {
                    self.y = v
                }
                self.nz(v, wide);
                int += 1;
            }
            Plb => {
                let v = self.pull8(bus);
                self.db = v;
                self.nz(v as u16, false);
                int += 1
            }
            Pld => {
                let v = self.pull16(bus);
                self.d = v;
                self.nz(v, true);
                int += 1
            }
            Plp => {
                let v = self.pull8(bus);
                self.set_p(v);
                int += 1
            }
            Rep => {
                let v = self.fetch8(bus);
                self.p &= !v;
                if self.e {
                    self.p |= 0x30
                }
            }
            Sep => {
                let v = self.fetch8(bus);
                self.set_p(self.p | v)
            }
            Clc => self.set_flag(C, false),
            Sec => self.set_flag(C, true),
            Cli => self.set_flag(I, false),
            Sei => self.set_flag(I, true),
            Cld => self.set_flag(D, false),
            Sed => self.set_flag(D, true),
            Clv => self.set_flag(V, false),
            Nop => {}
            Wdm => {
                let _ = self.fetch8(bus);
            }
            Wai => self.waiting = true,
            Stp => self.stopped = true,
            Xba => {
                self.a = self.a.swap_bytes();
                let v = self.a & 0xFF;
                self.nz(v, false);
                int += 1
            }
            Xce => {
                let c = self.flag(C);
                let was_e = self.e;
                self.set_flag(C, was_e);
                self.e = c;
                if self.e {
                    self.p |= 0x30;
                    self.x &= 0xFF;
                    self.y &= 0xFF;
                    self.s = 0x0100 | (self.s & 0xFF)
                }
            }
            Tax | Tay => {
                let wide = !self.x8();
                let v = if wide { self.a } else { self.a & 0xFF };
                if o == Tax {
                    self.x = v
                } else {
                    self.y = v
                }
                self.nz(v, wide);
            }
            Txa | Tya => {
                let wide = !self.m8();
                let v = if o == Txa { self.x } else { self.y };
                self.set_a(v, wide);
                self.nz(self.a_val(wide), wide);
            }
            Txy => {
                let v = self.x;
                self.y = v;
                let w = !self.x8();
                self.nz(v, w)
            }
            Tyx => {
                let v = self.y;
                self.x = v;
                let w = !self.x8();
                self.nz(v, w)
            }
            Tsx => {
                let w = !self.x8();
                let v = if w { self.s } else { self.s & 0xFF };
                self.x = v;
                self.nz(v, w)
            }
            Txs => {
                self.s = if self.e {
                    0x0100 | (self.x & 0xFF)
                } else {
                    self.x
                }
            }
            Tcs => {
                self.s = if self.e {
                    0x0100 | (self.a & 0xFF)
                } else {
                    self.a
                }
            }
            Tsc => {
                self.a = self.s;
                let v = self.a;
                self.nz(v, true)
            }
            Tcd => {
                self.d = self.a;
                let v = self.d;
                self.nz(v, true)
            }
            Tdc => {
                self.a = self.d;
                let v = self.a;
                self.nz(v, true)
            }
            Mvn | Mvp => {
                let src = self.fetch8(bus) as u32;
                let dst = self.fetch8(bus) as u32;
                self.db = dst as u8;
                let wide = !self.x8();
                let value = bus.read((src << 16) | self.x as u32);
                bus.write((dst << 16) | self.y as u32, value);
                let (delta_x, delta_y): (u16, u16) =
                    if o == Mvn { (1, 1) } else { (0xFFFF, 0xFFFF) };
                self.x = self.x.wrapping_add(delta_x);
                self.y = self.y.wrapping_add(delta_y);
                if !wide {
                    self.x &= 0xFF;
                    self.y &= 0xFF;
                }
                self.a = self.a.wrapping_sub(1);
                if self.a != 0xFFFF {
                    self.pc = self.pc.wrapping_sub(3);
                }
                int += 6;
            }
        }
        if self.e {
            self.s = 0x0100 | (self.s & 0xFF);
        }
        bus.cycles += int * 6;
        (bus.cycles - start) as u32
    }
}

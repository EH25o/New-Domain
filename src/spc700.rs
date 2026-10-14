//! Sony SPC700 interpreter and its memory-mapped timer/communication registers.

use crate::dsp::Dsp;

const N: u8 = 0x80;
const V: u8 = 0x40;
const P: u8 = 0x20;
const B: u8 = 0x10;
const H: u8 = 0x08;
const I: u8 = 0x04;
const Z: u8 = 0x02;
const C: u8 = 0x01;

const CYCLES: [u8; 256] = [
    2, 8, 4, 5, 3, 4, 3, 6, 2, 6, 5, 4, 5, 4, 6, 8, 2, 8, 4, 5, 4, 5, 5, 6, 5, 5, 6, 5, 5, 2, 4, 6,
    2, 8, 4, 6, 3, 4, 3, 6, 2, 6, 5, 4, 5, 4, 6, 4, 2, 8, 4, 6, 4, 5, 4, 6, 5, 5, 6, 5, 5, 2, 4, 8,
    2, 8, 4, 6, 3, 4, 3, 6, 2, 6, 5, 4, 5, 4, 6, 6, 2, 8, 4, 6, 4, 5, 4, 6, 5, 5, 6, 5, 5, 2, 4, 3,
    2, 8, 4, 6, 3, 4, 3, 6, 2, 6, 5, 4, 5, 4, 6, 6, 2, 8, 4, 6, 4, 5, 4, 6, 5, 5, 6, 5, 5, 2, 4, 3,
    2, 8, 4, 6, 3, 4, 3, 6, 2, 6, 5, 4, 5, 4, 6, 5, 2, 8, 4, 6, 4, 5, 4, 6, 5, 5, 6, 5, 5, 2, 4, 5,
    2, 8, 4, 6, 3, 4, 3, 6, 2, 6, 5, 4, 5, 4, 6, 5, 2, 8, 4, 6, 4, 5, 4, 6, 5, 5, 6, 5, 5, 2, 4, 5,
    2, 8, 4, 6, 3, 4, 3, 6, 2, 6, 5, 4, 5, 4, 6, 4, 2, 8, 4, 6, 4, 5, 4, 6, 5, 5, 6, 5, 5, 2, 4, 3,
    2, 8, 4, 6, 3, 4, 3, 6, 2, 6, 5, 4, 5, 4, 6, 4, 2, 8, 4, 6, 4, 5, 4, 6, 5, 5, 6, 5, 5, 2, 4, 3,
];

#[derive(Clone)]
pub struct Spc700 {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub psw: u8,
    pub ram: Vec<u8>,
    pub cycles: u64,
    pub cpu_to_spc: [u8; 4],
    pub spc_to_cpu: [u8; 4],
    pub dsp: Dsp,
    pub test: u8,
    pub control: u8,
    pub dsp_addr: u8,
    timer_target: [u8; 3],
    timer_divider: [u16; 3],
    timer_stage: [u16; 3],
    timer_output: [u8; 3],
    pub sleeping: bool,
    pub stopped: bool,
    #[cfg(test)]
    flat_test_memory: bool,
    #[cfg(test)]
    flat_test_touched: Vec<u16>,
}

impl Default for Spc700 {
    fn default() -> Self {
        Self::new()
    }
}

impl Spc700 {
    pub fn new() -> Spc700 {
        Spc700 {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xEF,
            pc: 0,
            psw: 0x02,
            ram: vec![0; 0x10000],
            cycles: 0,
            cpu_to_spc: [0; 4],
            spc_to_cpu: [0; 4],
            dsp: Dsp::new(),
            test: 0,
            control: 0,
            dsp_addr: 0,
            timer_target: [0; 3],
            timer_divider: [0; 3],
            timer_stage: [0; 3],
            timer_output: [0; 3],
            sleeping: false,
            stopped: false,
            #[cfg(test)]
            flat_test_memory: false,
            #[cfg(test)]
            flat_test_touched: Vec::new(),
        }
    }

    #[inline]
    fn flag(&self, flag: u8) -> bool {
        self.psw & flag != 0
    }

    fn opcode_cycles(opcode: u8) -> u8 {
        match opcode {
            0x1C | 0x2F | 0x3C | 0x5C | 0x7C | 0x9C | 0xBC | 0xDC | 0xFC => 2,
            0x3E | 0x7E => 3,
            0x4A | 0x6A => 4,
            0xAA => 4,
            0xCA => 6,
            0xCF => 9,
            0x5A => 4,
            0x7A | 0x9A => 5,
            0x8D | 0xAD | 0xCD => 2,
            0x8E | 0xAE | 0xCE | 0xEE => 4,
            0x9E => 12,
            0xA0 | 0xC0 => 3,
            0xAF | 0xBF => 4,
            0xBA | 0xDA => 5,
            0xBE | 0xDF => 3,
            0xC4 => 4,
            0xC5 => 5,
            0xC6 => 4,
            0xC7 => 7,
            0xC9 => 5,
            0xD4 => 5,
            0xD5 => 6,
            0xD6 => 6,
            0xD7 => 7,
            0xD8 => 4,
            0xDE => 6,
            0xE9 => 4,
            0xEB => 3,
            0xEC => 4,
            0xED => 3,
            0xEF => 7,
            0xFF => 7,
            0xF8 => 3,
            0xF9 => 4,
            0xFA => 5,
            0xFB => 4,
            0x6E => 5,
            0x6F => 5,
            0x7F => 6,
            0x36 | 0x56 | 0x76 | 0x96 | 0xB6 | 0xF6 => 5,
            0x03 | 0x13 | 0x23 | 0x33 | 0x43 | 0x53 | 0x63 | 0x73 | 0x83 | 0x93 | 0xA3 | 0xB3
            | 0xC3 | 0xD3 | 0xE3 | 0xF3 | 0x2E => 5,
            _ => CYCLES[opcode as usize],
        }
    }

    #[inline]
    fn set_flag(&mut self, flag: u8, set: bool) {
        if set {
            self.psw |= flag;
        } else {
            self.psw &= !flag;
        }
    }

    fn set_nz(&mut self, value: u8) {
        self.set_flag(N, value & 0x80 != 0);
        self.set_flag(Z, value == 0);
    }

    fn set_nz16(&mut self, value: u16) {
        self.set_flag(N, value & 0x8000 != 0);
        self.set_flag(Z, value == 0);
    }

    fn direct_page(&self, offset: u8) -> u16 {
        ((self.psw & P) as u16 * 8) | offset as u16
    }

    fn direct_next(&self, addr: u16) -> u16 {
        (addr & 0xFF00) | (addr.wrapping_add(1) & 0x00FF)
    }

    pub fn read(&mut self, addr: u16) -> u8 {
        #[cfg(test)]
        if self.flat_test_memory {
            return self.ram[addr as usize];
        }
        match addr {
            0x00F0 => self.test,
            0x00F1 => self.control,
            0x00F2 => self.dsp_addr,
            0x00F3 => self.dsp.read(self.dsp_addr),
            0x00F4..=0x00F7 => self.cpu_to_spc[(addr - 0x00F4) as usize],
            0x00F8..=0x00F9 => self.ram[addr as usize],
            0x00FA..=0x00FC => self.timer_target[(addr - 0x00FA) as usize],
            0x00FD..=0x00FF => {
                let timer = (addr - 0x00FD) as usize;
                let value = self.timer_output[timer] & 0x0F;
                self.timer_output[timer] = 0;
                value
            }
            _ => self.ram[addr as usize],
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        #[cfg(test)]
        if self.flat_test_memory {
            self.ram[addr as usize] = value;
            self.flat_test_touched.push(addr);
            return;
        }
        match addr {
            0x00F0 => self.test = value,
            0x00F1 => self.write_control(value),
            0x00F2 => self.dsp_addr = value,
            0x00F3 => self.dsp.write(self.dsp_addr, value),
            0x00F4..=0x00F7 => self.spc_to_cpu[(addr - 0x00F4) as usize] = value,
            0x00F8..=0x00F9 => self.ram[addr as usize] = value,
            0x00FA..=0x00FC => self.timer_target[(addr - 0x00FA) as usize] = value,
            0x00FD..=0x00FF => {}
            _ => self.ram[addr as usize] = value,
        }
    }

    fn write_control(&mut self, value: u8) {
        let rising = value & !self.control & 0x07;
        for timer in 0..3 {
            if rising & (1 << timer) != 0 {
                self.timer_divider[timer] = 0;
                self.timer_stage[timer] = 0;
                self.timer_output[timer] = 0;
            }
        }
        if value & 0x10 != 0 {
            self.cpu_to_spc[0] = 0;
            self.cpu_to_spc[1] = 0;
        }
        if value & 0x20 != 0 {
            self.cpu_to_spc[2] = 0;
            self.cpu_to_spc[3] = 0;
        }
        self.control = value;
    }

    pub fn set_cpu_port(&mut self, port: usize, value: u8) {
        self.cpu_to_spc[port & 3] = value;
    }

    pub fn cpu_port(&self, port: usize) -> u8 {
        self.spc_to_cpu[port & 3]
    }

    #[cfg(test)]
    pub(crate) fn begin_flat_test(&mut self) {
        for address in self.flat_test_touched.drain(..) {
            self.ram[address as usize] = 0;
        }
        self.flat_test_memory = true;
        self.cycles = 0;
    }

    #[cfg(test)]
    pub(crate) fn set_flat_test_byte(&mut self, address: u16, value: u8) {
        self.ram[address as usize] = value;
        self.flat_test_touched.push(address);
    }

    fn tick(&mut self, cycles: u8) {
        const PERIODS: [u16; 3] = [128, 128, 16];
        for (timer, &period) in PERIODS.iter().enumerate() {
            if self.control & (1 << timer) == 0 {
                continue;
            }
            self.timer_divider[timer] += cycles as u16;
            while self.timer_divider[timer] >= period {
                self.timer_divider[timer] -= period;
                self.timer_stage[timer] += 1;
                let target = if self.timer_target[timer] == 0 {
                    256
                } else {
                    self.timer_target[timer] as u16
                };
                if self.timer_stage[timer] >= target {
                    self.timer_stage[timer] = 0;
                    self.timer_output[timer] = self.timer_output[timer].wrapping_add(1) & 0x0F;
                }
            }
        }
        self.dsp.clock(cycles as u32);
        self.cycles += cycles as u64;
    }

    fn fetch8(&mut self) -> u8 {
        let value = self.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        value
    }

    fn fetch16(&mut self) -> u16 {
        let lo = self.fetch8() as u16;
        lo | (self.fetch8() as u16) << 8
    }

    fn read16(&mut self, addr: u16) -> u16 {
        let lo = self.read(addr) as u16;
        lo | (self.read(addr.wrapping_add(1)) as u16) << 8
    }

    fn read16_dp(&mut self, addr: u16) -> u16 {
        let lo = self.read(addr) as u16;
        lo | (self.read(self.direct_next(addr)) as u16) << 8
    }

    fn write16_dp(&mut self, addr: u16, value: u16) {
        self.write(addr, value as u8);
        self.write(self.direct_next(addr), (value >> 8) as u8);
    }

    fn fetch_dp_addr(&mut self) -> u16 {
        let offset = self.fetch8();
        self.direct_page(offset)
    }

    fn indirect_x_addr(&mut self) -> u16 {
        let dp = self.fetch8().wrapping_add(self.x);
        self.read16_dp(self.direct_page(dp))
    }

    fn indirect_y_addr(&mut self) -> u16 {
        let dp = self.fetch8();
        self.read16_dp(self.direct_page(dp))
            .wrapping_add(self.y as u16)
    }

    fn push(&mut self, value: u8) {
        self.write(0x0100 | self.sp as u16, value);
        self.sp = self.sp.wrapping_sub(1);
    }

    fn pop(&mut self) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        self.read(0x0100 | self.sp as u16)
    }

    fn push16(&mut self, value: u16) {
        self.push((value >> 8) as u8);
        self.push(value as u8);
    }

    fn pop16(&mut self) -> u16 {
        let lo = self.pop() as u16;
        lo | (self.pop() as u16) << 8
    }

    fn branch(&mut self, condition: bool) -> u8 {
        let offset = self.fetch8() as i8;
        if condition {
            self.pc = self.pc.wrapping_add(offset as i16 as u16);
            2
        } else {
            0
        }
    }

    fn adc_value(&mut self, a: u8, value: u8) -> u8 {
        let carry = self.flag(C) as u16;
        let sum = a as u16 + value as u16 + carry;
        let result = sum as u8;
        self.set_flag(H, (a & 0x0F) as u16 + (value & 0x0F) as u16 + carry > 0x0F);
        self.set_flag(V, (!(a ^ value) & (a ^ result) & 0x80) != 0);
        self.set_flag(C, sum > 0xFF);
        self.set_nz(result);
        result
    }

    fn sbc_value(&mut self, a: u8, value: u8) -> u8 {
        let borrow = (!self.flag(C)) as i16;
        let diff = a as i16 - value as i16 - borrow;
        let result = diff as u8;
        self.set_flag(H, (a & 0x0F) as i16 - (value & 0x0F) as i16 - borrow >= 0);
        self.set_flag(V, ((a ^ value) & (a ^ result) & 0x80) != 0);
        self.set_flag(C, diff >= 0);
        self.set_nz(result);
        result
    }

    fn alu(&mut self, kind: u8, value: u8) -> bool {
        let changed = match kind {
            0 => self.a | value,
            1 => self.a & value,
            2 => self.a ^ value,
            3 => {
                let result = self.a.wrapping_sub(value);
                self.set_flag(C, self.a >= value);
                self.set_nz(result);
                return false;
            }
            4 => self.adc_value(self.a, value),
            5 => self.sbc_value(self.a, value),
            _ => unreachable!(),
        };
        self.a = changed;
        self.set_nz(changed);
        true
    }

    fn memory_alu(&mut self, kind: u8, address: u16, value: u8) {
        let lhs = self.read(address);
        let result = match kind {
            0 => {
                let result = lhs | value;
                self.set_nz(result);
                result
            }
            1 => {
                let result = lhs & value;
                self.set_nz(result);
                result
            }
            2 => {
                let result = lhs ^ value;
                self.set_nz(result);
                result
            }
            3 => {
                let result = lhs.wrapping_sub(value);
                self.set_flag(C, lhs >= value);
                self.set_nz(result);
                return;
            }
            4 => self.adc_value(lhs, value),
            5 => self.sbc_value(lhs, value),
            _ => unreachable!(),
        };
        self.write(address, result);
    }

    fn compare(&mut self, lhs: u8, rhs: u8) {
        self.set_flag(C, lhs >= rhs);
        self.set_nz(lhs.wrapping_sub(rhs));
    }

    fn rmw(&mut self, address: u16, operation: u8) {
        let value = self.read(address);
        let result = match operation {
            0 => {
                self.set_flag(C, value & 0x80 != 0);
                value.wrapping_shl(1)
            }
            1 => {
                let carry = self.flag(C) as u8;
                self.set_flag(C, value & 0x80 != 0);
                (value << 1) | carry
            }
            2 => {
                self.set_flag(C, value & 1 != 0);
                value >> 1
            }
            3 => {
                let carry = (self.flag(C) as u8) << 7;
                self.set_flag(C, value & 1 != 0);
                (value >> 1) | carry
            }
            4 => value.wrapping_sub(1),
            5 => value.wrapping_add(1),
            _ => unreachable!(),
        };
        self.set_nz(result);
        self.write(address, result);
    }

    fn bit_operand(&mut self) -> (u16, u8) {
        let operand = self.fetch16();
        (operand & 0x1FFF, 1 << (operand >> 13))
    }

    fn execute_alu(&mut self, opcode: u8) -> bool {
        let kind = opcode >> 5;
        if kind > 5 {
            return false;
        }
        let value = match opcode & 0x1F {
            0x04 => {
                let address = self.fetch_dp_addr();
                self.read(address)
            }
            0x05 => {
                let address = self.fetch16();
                self.read(address)
            }
            0x06 => self.read(self.direct_page(self.x)),
            0x07 => {
                let address = self.indirect_x_addr();
                self.read(address)
            }
            0x08 => self.fetch8(),
            0x14 => {
                let address = self.fetch8().wrapping_add(self.x);
                self.read(self.direct_page(address))
            }
            0x15 => {
                let address = self.fetch16().wrapping_add(self.x as u16);
                self.read(address)
            }
            0x16 => {
                let address = self.fetch16().wrapping_add(self.y as u16);
                self.read(address)
            }
            0x17 => {
                let address = self.indirect_y_addr();
                self.read(address)
            }
            _ => return false,
        };
        self.alu(kind, value);
        true
    }

    fn alu_memory_pair(&mut self, kind: u8) {
        let source = self.fetch_dp_addr();
        let destination = self.fetch_dp_addr();
        let value = self.read(source);
        self.memory_alu(kind, destination, value);
    }

    fn alu_memory_immediate(&mut self, kind: u8) {
        let value = self.fetch8();
        let destination = self.fetch_dp_addr();
        self.memory_alu(kind, destination, value);
    }

    fn add_word(&mut self, value: u16, subtract: bool, compare: bool) {
        let ya = (self.y as u16) << 8 | self.a as u16;
        let result = if subtract {
            let diff = ya as i32 - value as i32;
            self.set_flag(C, diff >= 0);
            let result = diff as u16;
            if !compare {
                self.set_flag(H, (ya & 0x0FFF) >= (value & 0x0FFF));
                self.set_flag(V, ((ya ^ value) & (ya ^ result) & 0x8000) != 0);
            }
            result
        } else {
            let sum = ya as u32 + value as u32;
            self.set_flag(C, sum > 0xFFFF);
            self.set_flag(H, (ya & 0x0FFF) as u32 + (value & 0x0FFF) as u32 > 0x0FFF);
            let result = sum as u16;
            self.set_flag(V, (!(ya ^ value) & (ya ^ result) & 0x8000) != 0);
            result
        };
        self.set_nz16(ya.wrapping_sub(value));
        if !compare {
            self.a = result as u8;
            self.y = (result >> 8) as u8;
            self.set_nz16(result);
        }
    }

    fn bit_branch(&mut self, opcode: u8, set: bool) -> u8 {
        let address = self.fetch_dp_addr();
        let value = self.read(address);
        let offset = self.fetch8() as i8;
        let bit = 1 << (opcode >> 5);
        if (value & bit != 0) == set {
            self.pc = self.pc.wrapping_add(offset as i16 as u16);
            2
        } else {
            0
        }
    }

    fn decimal_adjust(&mut self, subtract: bool) {
        let correction = if subtract {
            if !self.flag(C) || self.a > 0x99 {
                self.set_flag(C, false);
                0x60
            } else {
                0
            }
        } else if self.flag(C) || self.a > 0x99 {
            self.set_flag(C, true);
            0x60
        } else {
            0
        };
        let mut result = if subtract {
            self.a.wrapping_sub(correction)
        } else {
            self.a.wrapping_add(correction)
        };
        if (subtract && (!self.flag(H) || result & 0x0F > 9))
            || (!subtract && (self.flag(H) || result & 0x0F > 9))
        {
            result = if subtract {
                result.wrapping_sub(0x06)
            } else {
                result.wrapping_add(0x06)
            };
        }
        self.a = result;
        self.set_nz(result);
    }

    fn execute_special(&mut self, opcode: u8) -> Option<u8> {
        let extra = match opcode {
            0x03 | 0x23 | 0x43 | 0x63 | 0x83 | 0xA3 | 0xC3 | 0xE3 => {
                return Some(self.bit_branch(opcode, true))
            }
            0x13 | 0x33 | 0x53 | 0x73 | 0x93 | 0xB3 | 0xD3 | 0xF3 => {
                return Some(self.bit_branch(opcode, false))
            }
            0x0A | 0x2A | 0x4A | 0x6A | 0x8A | 0xAA | 0xCA | 0xEA => {
                let (address, mask) = self.bit_operand();
                let bit = self.read(address) & mask != 0;
                match opcode {
                    0x0A => self.set_flag(C, self.flag(C) | bit),
                    0x2A => self.set_flag(C, self.flag(C) | !bit),
                    0x4A => self.set_flag(C, self.flag(C) & bit),
                    0x6A => self.set_flag(C, self.flag(C) & !bit),
                    0x8A => self.set_flag(C, self.flag(C) ^ bit),
                    0xAA => self.set_flag(C, bit),
                    0xCA => {
                        let value = self.read(address);
                        self.write(
                            address,
                            if self.flag(C) {
                                value | mask
                            } else {
                                value & !mask
                            },
                        );
                    }
                    0xEA => {
                        let value = self.read(address);
                        self.write(address, value ^ mask);
                    }
                    _ => unreachable!(),
                }
                0
            }
            0x09 | 0x29 | 0x49 | 0x69 | 0x89 | 0xA9 => {
                self.alu_memory_pair(opcode >> 5);
                0
            }
            0x18 | 0x38 | 0x58 | 0x78 | 0x98 | 0xB8 => {
                self.alu_memory_immediate(opcode >> 5);
                0
            }
            0x19 | 0x39 | 0x59 | 0x79 | 0x99 | 0xB9 => {
                let value = self.read(self.direct_page(self.y));
                self.memory_alu(opcode >> 5, self.direct_page(self.x), value);
                0
            }
            0x1A | 0x3A => {
                let address = self.fetch_dp_addr();
                let value =
                    self.read16_dp(address)
                        .wrapping_add(if opcode == 0x1A { u16::MAX } else { 1 });
                self.write16_dp(address, value);
                self.set_nz16(value);
                0
            }
            0x1F => {
                let address = self.fetch16().wrapping_add(self.x as u16);
                self.pc = self.read16(address);
                0
            }
            0x2E => {
                let address = self.fetch_dp_addr();
                let value = self.read(address);
                let offset = self.fetch8() as i8;
                if value != self.a {
                    self.pc = self.pc.wrapping_add(offset as i16 as u16);
                    2
                } else {
                    0
                }
            }
            0x6E => {
                let address = self.fetch_dp_addr();
                let value = self.read(address).wrapping_sub(1);
                self.write(address, value);
                let offset = self.fetch8() as i8;
                if value != 0 {
                    self.pc = self.pc.wrapping_add(offset as i16 as u16);
                    2
                } else {
                    0
                }
            }
            0xDE => {
                let offset = self.fetch8().wrapping_add(self.x);
                let address = self.direct_page(offset);
                let value = self.read(address);
                let relative = self.fetch8() as i8;
                if value != self.a {
                    self.pc = self.pc.wrapping_add(relative as i16 as u16);
                    2
                } else {
                    0
                }
            }
            0x0E | 0x4E => {
                let address = self.fetch16();
                let value = self.read(address);
                self.set_nz(self.a.wrapping_sub(value));
                self.write(
                    address,
                    if opcode == 0x0E {
                        value | self.a
                    } else {
                        value & !self.a
                    },
                );
                0
            }
            0x0B | 0x2B | 0x4B | 0x6B | 0x8B | 0xAB | 0x0C | 0x2C | 0x4C | 0x6C | 0x8C | 0xAC => {
                let address = if opcode & 0x0F == 0x0B {
                    self.fetch_dp_addr()
                } else {
                    self.fetch16()
                };
                let operation = match opcode >> 5 {
                    0 => 0,
                    1 => 1,
                    2 => 2,
                    3 => 3,
                    4 => 4,
                    5 => 5,
                    _ => unreachable!(),
                };
                self.rmw(address, operation);
                0
            }
            0x1B | 0x3B | 0x5B | 0x7B | 0x9B | 0xBB => {
                let offset = self.fetch8().wrapping_add(self.x);
                let address = self.direct_page(offset);
                self.rmw(address, (opcode - 0x1B) / 0x20);
                0
            }
            0x1C | 0x3C | 0x5C | 0x7C | 0x9C | 0xBC => {
                let old = self.a;
                let operation = if opcode == 0xFC { 5 } else { opcode >> 5 };
                let result = match operation {
                    0 => {
                        self.set_flag(C, old & 0x80 != 0);
                        old.wrapping_shl(1)
                    }
                    1 => {
                        let c = self.flag(C) as u8;
                        self.set_flag(C, old & 0x80 != 0);
                        (old << 1) | c
                    }
                    2 => {
                        self.set_flag(C, old & 1 != 0);
                        old >> 1
                    }
                    3 => {
                        let c = (self.flag(C) as u8) << 7;
                        self.set_flag(C, old & 1 != 0);
                        (old >> 1) | c
                    }
                    4 => old.wrapping_sub(1),
                    5 => old.wrapping_add(1),
                    _ => unreachable!(),
                };
                self.a = result;
                self.set_nz(result);
                0
            }
            0x02 | 0x22 | 0x42 | 0x62 | 0x82 | 0xA2 | 0xC2 | 0xE2 | 0x12 | 0x32 | 0x52 | 0x72
            | 0x92 | 0xB2 | 0xD2 | 0xF2 => {
                let address = self.fetch_dp_addr();
                let mut value = self.read(address);
                let bit = 1 << (opcode >> 5);
                if opcode & 0x10 == 0 {
                    value |= bit;
                } else {
                    value &= !bit;
                }
                self.write(address, value);
                0
            }
            0x10 => return Some(self.branch(!self.flag(N))),
            0x30 => return Some(self.branch(self.flag(N))),
            0x50 => return Some(self.branch(!self.flag(V))),
            0x70 => return Some(self.branch(self.flag(V))),
            0x90 => return Some(self.branch(!self.flag(C))),
            0xB0 => return Some(self.branch(self.flag(C))),
            0xD0 => return Some(self.branch(!self.flag(Z))),
            0xF0 => return Some(self.branch(self.flag(Z))),
            0x2F => return Some(self.branch(true)),
            0x0F => {
                self.push16(self.pc);
                self.push(self.psw);
                self.set_flag(B, true);
                self.set_flag(I, false);
                self.pc = self.read16(0xFFDE);
                0
            }
            0x3F => {
                let target = self.fetch16();
                self.push16(self.pc);
                self.pc = target;
                0
            }
            0x4F => {
                let low = self.fetch8();
                self.push16(self.pc);
                self.pc = 0xFF00 | low as u16;
                0
            }
            0x5F => {
                self.pc = self.fetch16();
                0
            }
            0x6F => {
                self.pc = self.pop16();
                0
            }
            0x7F => {
                self.psw = self.pop();
                self.pc = self.pop16();
                0
            }
            0x0D | 0x2D | 0x4D | 0x6D => {
                let value = match opcode {
                    0x0D => self.psw,
                    0x2D => self.a,
                    0x4D => self.x,
                    _ => self.y,
                };
                self.push(value);
                0
            }
            0x8D => {
                self.y = self.fetch8();
                self.set_nz(self.y);
                0
            }
            0xCD => {
                self.x = self.fetch8();
                self.set_nz(self.x);
                0
            }
            0x8E | 0xAE | 0xCE | 0xEE => {
                let value = self.pop();
                match opcode {
                    0x8E => self.psw = value,
                    0xAE => self.a = value,
                    0xCE => self.x = value,
                    _ => self.y = value,
                }
                0
            }
            0x01 | 0x11 | 0x31 | 0x51 | 0x71 | 0x91 | 0xB1 | 0xD1 | 0xF1 | 0x21 | 0x41 | 0x61
            | 0x81 | 0xA1 | 0xC1 | 0xE1 => {
                let vector = 0xFFDEu16.wrapping_sub(((opcode >> 4) as u16) * 2);
                self.push16(self.pc);
                self.pc = self.read16(vector);
                0
            }
            0x1E | 0x5E => {
                let address = self.fetch16();
                let value = self.read(address);
                self.compare(if opcode == 0x1E { self.x } else { self.y }, value);
                0
            }
            0x3E | 0x7E => {
                let address = self.fetch_dp_addr();
                let value = self.read(address);
                self.compare(if opcode == 0x3E { self.x } else { self.y }, value);
                0
            }
            0xAD => {
                let value = self.fetch8();
                self.compare(self.y, value);
                0
            }
            0xC8 => {
                let value = self.fetch8();
                self.compare(self.x, value);
                0
            }
            0x1D | 0x3D | 0xDC | 0xFC => {
                let result = if opcode == 0x1D {
                    self.x = self.x.wrapping_sub(1);
                    self.x
                } else if opcode == 0xDC {
                    self.y = self.y.wrapping_sub(1);
                    self.y
                } else if opcode == 0xFC {
                    self.y = self.y.wrapping_add(1);
                    self.y
                } else {
                    self.x = self.x.wrapping_add(1);
                    self.x
                };
                self.set_nz(result);
                0
            }
            0x20 | 0x40 | 0x60 | 0x80 | 0xA0 | 0xC0 | 0xE0 => {
                let flag = match opcode {
                    0x20 => P,
                    0x40 => P,
                    0x60 => C,
                    0x80 => C,
                    0xA0 => I,
                    0xC0 => I,
                    0xE0 => V,
                    _ => unreachable!(),
                };
                match opcode {
                    0x20 | 0x60 | 0xC0 => self.set_flag(flag, false),
                    0x40 | 0x80 | 0xA0 => self.set_flag(flag, true),
                    0xE0 => {
                        self.set_flag(V, false);
                        self.set_flag(H, false);
                    }
                    _ => {}
                }
                0
            }
            0x9D => {
                self.x = self.sp;
                self.set_nz(self.x);
                0
            }
            0xED => {
                self.set_flag(C, !self.flag(C));
                0
            }
            0xBD => {
                self.sp = self.x;
                0
            }
            0x5D => {
                self.x = self.a;
                self.set_nz(self.x);
                0
            }
            0x7D => {
                self.a = self.x;
                self.set_nz(self.a);
                0
            }
            0xDD => {
                self.a = self.y;
                self.set_nz(self.a);
                0
            }
            0xFD => {
                self.y = self.a;
                self.set_nz(self.y);
                0
            }
            0x9F => {
                self.a = self.a.rotate_left(4);
                self.set_nz(self.a);
                0
            }
            0xBE => {
                self.decimal_adjust(true);
                0
            }
            0xDF => {
                self.decimal_adjust(false);
                0
            }
            0xCF => {
                let result = self.a as u16 * self.y as u16;
                self.a = result as u8;
                self.y = (result >> 8) as u8;
                self.set_nz(self.y);
                0
            }
            0x9E => {
                let y = self.y;
                let ya = (self.y as u16) << 8 | self.a as u16;
                let x = self.x as u16;
                let overflow = self.y >= self.x;
                if (self.y as u16) < x * 2 {
                    self.a = (ya / x) as u8;
                    self.y = (ya % x) as u8;
                } else {
                    let numerator = ya.wrapping_sub(x << 9);
                    let divisor = 0x100 - x;
                    self.a = 0xFFu16.wrapping_sub(numerator / divisor) as u8;
                    self.y = (x + numerator % divisor) as u8;
                }
                self.set_flag(H, (y & 0x0F) >= (self.x & 0x0F));
                self.set_flag(V, overflow);
                self.set_nz(self.a);
                0
            }
            0xBA | 0xDA => {
                let address = self.fetch_dp_addr();
                if opcode == 0xBA {
                    let ya = self.read16_dp(address);
                    self.a = ya as u8;
                    self.y = (ya >> 8) as u8;
                    self.set_nz16(ya);
                } else {
                    let ya = (self.y as u16) << 8 | self.a as u16;
                    self.write16_dp(address, ya);
                }
                0
            }
            0x5A | 0x7A | 0x9A => {
                let address = self.fetch_dp_addr();
                let value = self.read16_dp(address);
                self.add_word(value, opcode != 0x7A, opcode == 0x5A);
                0
            }
            0xAF | 0xBF => {
                let address = self.direct_page(self.x);
                if opcode == 0xAF {
                    self.write(address, self.a);
                    self.x = self.x.wrapping_add(1);
                } else {
                    self.a = self.read(address);
                    self.x = self.x.wrapping_add(1);
                    self.set_nz(self.a);
                }
                0
            }
            0xFE => {
                self.y = self.y.wrapping_sub(1);
                self.branch(self.y != 0)
            }
            0xEF => {
                self.sleeping = true;
                0
            }
            0xFF => {
                self.stopped = true;
                0
            }
            0x00 => 0,
            _ => return None,
        };
        Some(extra)
    }

    fn execute_move(&mut self, opcode: u8) -> bool {
        match opcode {
            0x04 | 0x14 | 0xF4 => {
                let offset = self
                    .fetch8()
                    .wrapping_add(if opcode == 0x04 { 0 } else { self.x });
                self.a = self.read(self.direct_page(offset));
                self.set_nz(self.a);
            }
            0x05 | 0x15 | 0xF5 | 0x16 | 0xF6 => {
                let base = self.fetch16();
                let address = base.wrapping_add(match opcode {
                    0x15 | 0xF5 => self.x as u16,
                    0x16 | 0xF6 => self.y as u16,
                    _ => 0,
                });
                self.a = self.read(address);
                self.set_nz(self.a);
            }
            0x06 => {
                self.a = self.read(self.direct_page(self.x));
                self.set_nz(self.a);
            }
            0x07 | 0x17 | 0xF7 => {
                let address = if opcode == 0x07 {
                    self.indirect_x_addr()
                } else {
                    self.indirect_y_addr()
                };
                self.a = self.read(address);
                self.set_nz(self.a);
            }
            0x08 | 0xE8 => {
                self.a = self.fetch8();
                self.set_nz(self.a);
            }
            0xE4 | 0xF8 | 0xEB | 0xF9 | 0xFB => {
                let index = if opcode == 0xF9 {
                    self.y
                } else if opcode == 0xFB {
                    self.x
                } else {
                    0
                };
                let offset = self.fetch8().wrapping_add(index);
                let address = self.direct_page(offset);
                let value = self.read(address);
                match opcode {
                    0xE4 => self.a = value,
                    0xF8 | 0xF9 => self.x = value,
                    _ => self.y = value,
                }
                self.set_nz(value);
            }
            0xE5 | 0xE9 | 0xEC => {
                let address = self.fetch16();
                let value = self.read(address);
                match opcode {
                    0xE5 => self.a = value,
                    0xE9 => self.x = value,
                    _ => self.y = value,
                }
                self.set_nz(value);
            }
            0xE6 => {
                self.a = self.read(self.direct_page(self.x));
                self.set_nz(self.a);
            }
            0xE7 => {
                let address = self.indirect_x_addr();
                self.a = self.read(address);
                self.set_nz(self.a);
            }
            0xC4 | 0xD4 | 0xDB => {
                let offset = self
                    .fetch8()
                    .wrapping_add(if opcode == 0xC4 { 0 } else { self.x });
                let value = if opcode == 0xDB { self.y } else { self.a };
                self.write(self.direct_page(offset), value);
            }
            0xC5 | 0xD5 | 0xC9 | 0xCC => {
                let base = self.fetch16();
                let address = base.wrapping_add(if opcode == 0xD5 { self.x as u16 } else { 0 });
                let value = match opcode {
                    0xC9 => self.x,
                    0xCC => self.y,
                    _ => self.a,
                };
                self.write(address, value);
            }
            0xC6 => self.write(self.direct_page(self.x), self.a),
            0xC7 | 0xD7 => {
                let address = if opcode == 0xC7 {
                    self.indirect_x_addr()
                } else {
                    self.indirect_y_addr()
                };
                self.write(address, self.a);
            }
            0xD6 => {
                let address = self.fetch16().wrapping_add(self.y as u16);
                self.write(address, self.a);
            }
            0xD8 | 0xD9 | 0xCB => {
                let offset = self
                    .fetch8()
                    .wrapping_add(if opcode == 0xD9 { self.y } else { 0 });
                self.write(
                    self.direct_page(offset),
                    if opcode == 0xCB { self.y } else { self.x },
                );
            }
            0x8F => {
                let value = self.fetch8();
                let address = self.fetch_dp_addr();
                self.write(address, value);
            }
            0x89 | 0xA9 | 0xFA => {
                let source = self.fetch_dp_addr();
                let destination = self.fetch_dp_addr();
                let value = self.read(source);
                self.write(destination, value);
            }
            0xCD => {
                self.x = self.fetch8();
                self.set_nz(self.x);
            }
            _ => return false,
        }
        true
    }

    pub fn step(&mut self) -> u8 {
        if self.stopped || self.sleeping {
            self.tick(1);
            return 1;
        }
        let opcode = self.fetch8();
        let mut cycles = Self::opcode_cycles(opcode);

        let extra = if self.execute_alu(opcode) {
            0
        } else if let Some(extra) = self.execute_special(opcode) {
            extra
        } else if self.execute_move(opcode) {
            0
        } else {
            panic!("unimplemented SPC700 opcode ${opcode:02X}");
        };
        cycles += extra;
        self.tick(cycles);
        cycles
    }
}

#[cfg(test)]
mod io_tests {
    use super::*;

    #[test]
    fn cpu_and_spc_ports_are_separate_latches() {
        let mut spc = Spc700::new();
        spc.set_cpu_port(0, 0xA5);
        assert_eq!(spc.read(0xF4), 0xA5);
        assert_eq!(spc.cpu_port(0), 0);

        spc.write(0xF4, 0x5A);
        assert_eq!(spc.cpu_port(0), 0x5A);
        assert_eq!(spc.read(0xF4), 0xA5);
    }

    #[test]
    fn timer_counts_at_the_configured_period_and_reads_clear_it() {
        let mut spc = Spc700::new();
        spc.write(0xFA, 1);
        spc.write(0xF1, 1);
        spc.tick(127);
        assert_eq!(spc.read(0xFD), 0);
        spc.tick(1);
        assert_eq!(spc.read(0xFD), 1);
        assert_eq!(spc.read(0xFD), 0);
    }

    #[test]
    fn control_clears_port_pairs_and_restarts_enabled_timers() {
        let mut spc = Spc700::new();
        spc.set_cpu_port(0, 1);
        spc.set_cpu_port(1, 2);
        spc.set_cpu_port(2, 3);
        spc.set_cpu_port(3, 4);
        spc.write(0xFA, 1);
        spc.write(0xF1, 0x11);
        assert_eq!(&spc.cpu_to_spc, &[0, 0, 3, 4]);

        spc.tick(64);
        spc.write(0xF1, 0);
        spc.write(0xF1, 1);
        spc.tick(127);
        assert_eq!(spc.read(0xFD), 0);
        spc.tick(1);
        assert_eq!(spc.read(0xFD), 1);
    }
}

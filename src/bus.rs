//! Barramento do SNES: mapa de memória LoROM, registradores de hardware e DMA.

use crate::apu::Apu;
use crate::ppu::Ppu;
use crate::rom::Rom;
use std::collections::HashMap;

#[derive(Default)]
pub struct Stats {
    pub dma_transfers: u64,
    pub dma_bytes: u64,
    pub dma_to_vram: u64,
    pub dma_to_cgram: u64,
    pub dma_to_oam: u64,
    pub dma_to_other: u64,
    pub hdma_enabled_mask: u8,
    pub unknown_reads: HashMap<u16, u32>,
    pub unknown_writes: HashMap<u16, u32>,
}

pub struct Bus {
    pub rom: Rom,
    pub wram: Vec<u8>,
    pub ppu: Ppu,
    pub apu: Apu,
    pub cycles: u64,
    mdr: u8,
    #[cfg(test)]
    flat_memory: Option<Vec<u8>>,
    #[cfg(test)]
    flat_touched: Vec<usize>,

    // registradores da CPU ($4200-$421F)
    pub nmitimen: u8,
    pub nmi_flag: bool,
    pub nmi_prev: bool,
    pub irq_flag: bool,
    pub htime: u16,
    pub vtime: u16,
    pub memsel: u8,
    wrmpya: u8,
    wrdiv: u16,
    rddiv: u16,
    rdmpy: u16,
    wmadd: u32,

    // joypads
    pub joy: [u16; 2],
    joy_auto: [u16; 4],
    joy_latch: [u16; 2],
    joy_strobe: bool,
    pub autojoy_busy: bool,

    // posição de vídeo (atualizada pelo laço principal)
    pub hclk: u32,
    pub vcount: u16,

    pub dma: [[u8; 16]; 8],
    pub stats: Stats,
}

impl Bus {
    pub fn new(rom: Rom) -> Bus {
        Bus {
            rom,
            wram: vec![0; 0x20000],
            ppu: Ppu::new(),
            apu: Apu::new(),
            cycles: 0,
            mdr: 0,
            #[cfg(test)]
            flat_memory: None,
            #[cfg(test)]
            flat_touched: Vec::new(),
            nmitimen: 0,
            nmi_flag: false,
            nmi_prev: false,
            irq_flag: false,
            htime: 0x1FF,
            vtime: 0x1FF,
            memsel: 0,
            wrmpya: 0xFF,
            wrdiv: 0xFFFF,
            rddiv: 0,
            rdmpy: 0,
            wmadd: 0,
            joy: [0; 2],
            joy_auto: [0; 4],
            joy_latch: [0; 2],
            joy_strobe: false,
            autojoy_busy: false,
            hclk: 0,
            vcount: 0,
            dma: [[0xFF; 16]; 8],
            stats: Stats::default(),
        }
    }

    // ---------- acesso ----------
    pub fn read(&mut self, addr: u32) -> u8 {
        #[cfg(test)]
        if let Some(memory) = &self.flat_memory {
            let value = memory[(addr & 0xFF_FFFF) as usize];
            self.mdr = value;
            self.cycles += 6;
            return value;
        }
        let (v, cost) = self.read_inner(addr);
        self.mdr = v;
        self.cycles += cost;
        v
    }

    fn rom_cost(&self, bank: u8) -> u64 {
        if bank >= 0x80 && self.memsel & 1 != 0 {
            6
        } else {
            8
        }
    }

    fn read_inner(&mut self, addr: u32) -> (u8, u64) {
        let bank = (addr >> 16) as u8;
        let off = addr as u16;
        match bank {
            0x7E => (self.wram[off as usize], 8),
            0x7F => (self.wram[0x10000 + off as usize], 8),
            0x00..=0x3F | 0x80..=0xBF => match off {
                0x0000..=0x1FFF => (self.wram[off as usize], 8),
                0x2100..=0x213F => match self.ppu.read(off as u8) {
                    Some(v) => (v, 6),
                    None => {
                        self.log_unknown_read(off);
                        (self.mdr, 6)
                    }
                },
                0x2140..=0x217F => (self.apu.read((off & 3) as usize), 6),
                0x2180 => {
                    let v = self.wram[(self.wmadd & 0x1FFFF) as usize];
                    self.wmadd = (self.wmadd + 1) & 0x1FFFF;
                    (v, 6)
                }
                0x4016 => (self.joy_serial(0), 12),
                0x4017 => (self.joy_serial(1) | 0x1C, 12),
                0x4210 => {
                    let v = (self.nmi_flag as u8) << 7 | 0x02;
                    self.nmi_flag = false;
                    (v | (self.mdr & 0x70), 6)
                }
                0x4211 => {
                    let v = (self.irq_flag as u8) << 7;
                    self.irq_flag = false;
                    (v | (self.mdr & 0x7F), 6)
                }
                0x4212 => {
                    let vblank = self.vcount >= 225;
                    let hblank = self.hclk >= 1096;
                    (
                        (vblank as u8) << 7 | (hblank as u8) << 6 | self.autojoy_busy as u8,
                        6,
                    )
                }
                0x4213 => (0xFF, 6),
                0x4214 => (self.rddiv as u8, 6),
                0x4215 => ((self.rddiv >> 8) as u8, 6),
                0x4216 => (self.rdmpy as u8, 6),
                0x4217 => ((self.rdmpy >> 8) as u8, 6),
                0x4218..=0x421F => {
                    let w = self.joy_auto[((off - 0x4218) / 2) as usize];
                    (
                        if off & 1 == 0 {
                            w as u8
                        } else {
                            (w >> 8) as u8
                        },
                        6,
                    )
                }
                0x4300..=0x437F => (self.dma[((off >> 4) & 7) as usize][(off & 0xF) as usize], 6),
                0x8000..=0xFFFF => (
                    self.rom.read(bank, off).unwrap_or(self.mdr),
                    self.rom_cost(bank),
                ),
                _ => {
                    self.log_unknown_read(off);
                    (self.mdr, 6)
                }
            },
            _ => (self.rom.read(bank, off).unwrap_or(self.mdr), 8),
        }
    }

    pub fn write(&mut self, addr: u32, v: u8) {
        #[cfg(test)]
        if self.flat_memory.is_some() {
            let address = (addr & 0xFF_FFFF) as usize;
            self.flat_memory.as_mut().expect("flat memory")[address] = v;
            self.flat_touched.push(address);
            self.mdr = v;
            self.cycles += 6;
            return;
        }
        let cost = self.write_inner(addr, v);
        self.mdr = v;
        self.cycles += cost;
    }

    #[cfg(test)]
    pub(crate) fn enable_flat_test_memory(&mut self) {
        self.flat_memory = Some(vec![0; 0x100_0000]);
    }

    #[cfg(test)]
    pub(crate) fn reset_flat_test_memory(&mut self) {
        if let Some(memory) = &mut self.flat_memory {
            for address in self.flat_touched.drain(..) {
                memory[address] = 0;
            }
        }
        self.cycles = 0;
    }

    #[cfg(test)]
    pub(crate) fn set_flat_test_byte(&mut self, address: u32, value: u8) {
        let address = (address & 0xFF_FFFF) as usize;
        self.flat_memory.as_mut().expect("flat test memory")[address] = value;
        self.flat_touched.push(address);
    }

    #[cfg(test)]
    pub(crate) fn flat_test_byte(&self, address: u32) -> u8 {
        self.flat_memory.as_ref().expect("flat test memory")[(address & 0xFF_FFFF) as usize]
    }

    fn write_inner(&mut self, addr: u32, v: u8) -> u64 {
        let bank = (addr >> 16) as u8;
        let off = addr as u16;
        match bank {
            0x7E => {
                self.wram[off as usize] = v;
                8
            }
            0x7F => {
                self.wram[0x10000 + off as usize] = v;
                8
            }
            0x00..=0x3F | 0x80..=0xBF => match off {
                0x0000..=0x1FFF => {
                    self.wram[off as usize] = v;
                    8
                }
                0x2100..=0x213F => {
                    self.ppu.write(off as u8, v);
                    6
                }
                0x2140..=0x217F => {
                    self.apu.write((off & 3) as usize, v);
                    6
                }
                0x2180 => {
                    self.wram[(self.wmadd & 0x1FFFF) as usize] = v;
                    self.wmadd = (self.wmadd + 1) & 0x1FFFF;
                    6
                }
                0x2181 => {
                    self.wmadd = (self.wmadd & 0x1FF00) | v as u32;
                    6
                }
                0x2182 => {
                    self.wmadd = (self.wmadd & 0x100FF) | (v as u32) << 8;
                    6
                }
                0x2183 => {
                    self.wmadd = (self.wmadd & 0x0FFFF) | ((v as u32 & 1) << 16);
                    6
                }
                0x4016 => {
                    let strobe = v & 1 != 0;
                    if self.joy_strobe && !strobe {
                        self.joy_latch = self.joy;
                    }
                    self.joy_strobe = strobe;
                    if strobe {
                        self.joy_latch = self.joy
                    }
                    12
                }
                0x4200 => {
                    self.nmitimen = v;
                    if v & 0x30 == 0 {
                        self.irq_flag = false
                    }
                    6
                }
                0x4201 => 6,
                0x4202 => {
                    self.wrmpya = v;
                    6
                }
                0x4203 => {
                    self.rdmpy = self.wrmpya as u16 * v as u16;
                    6
                }
                0x4204 => {
                    self.wrdiv = (self.wrdiv & 0xFF00) | v as u16;
                    6
                }
                0x4205 => {
                    self.wrdiv = (self.wrdiv & 0x00FF) | (v as u16) << 8;
                    6
                }
                0x4206 => {
                    if v == 0 {
                        self.rddiv = 0xFFFF;
                        self.rdmpy = self.wrdiv
                    } else {
                        self.rddiv = self.wrdiv / v as u16;
                        self.rdmpy = self.wrdiv % v as u16
                    }
                    6
                }
                0x4207 => {
                    self.htime = (self.htime & 0x100) | v as u16;
                    6
                }
                0x4208 => {
                    self.htime = (self.htime & 0x0FF) | ((v as u16 & 1) << 8);
                    6
                }
                0x4209 => {
                    self.vtime = (self.vtime & 0x100) | v as u16;
                    6
                }
                0x420A => {
                    self.vtime = (self.vtime & 0x0FF) | ((v as u16 & 1) << 8);
                    6
                }
                0x420B => {
                    self.run_dma(v);
                    6
                }
                0x420C => {
                    self.stats.hdma_enabled_mask |= v;
                    6
                }
                0x420D => {
                    self.memsel = v;
                    6
                }
                0x4300..=0x437F => {
                    self.dma[((off >> 4) & 7) as usize][(off & 0xF) as usize] = v;
                    6
                }
                0x8000..=0xFFFF => 8, // ROM: escrita ignorada
                _ => {
                    self.log_unknown_write(off);
                    6
                }
            },
            _ => 8,
        }
    }

    fn log_unknown_read(&mut self, off: u16) {
        *self.stats.unknown_reads.entry(off).or_default() += 1
    }
    fn log_unknown_write(&mut self, off: u16) {
        *self.stats.unknown_writes.entry(off).or_default() += 1
    }

    fn joy_serial(&mut self, port: usize) -> u8 {
        if self.joy_strobe {
            return (self.joy[port] >> 15) as u8 & 1;
        }
        let bit = (self.joy_latch[port] >> 15) as u8 & 1;
        self.joy_latch[port] = (self.joy_latch[port] << 1) | 1;
        bit
    }

    /// Leitura automática do controle (no início do vblank).
    pub fn latch_auto_joypad(&mut self) {
        self.joy_auto = [self.joy[0], self.joy[1], 0, 0];
    }

    // ---------- DMA de uso geral ----------
    fn read_a(&mut self, bank: u8, a: u16) -> u8 {
        if matches!(bank, 0x00..=0x3F | 0x80..=0xBF) && (0x2100..=0x21FF).contains(&a) {
            return 0; // o barramento A não alcança o barramento B
        }
        self.read_inner(((bank as u32) << 16) | a as u32).0
    }

    fn run_dma(&mut self, mask: u8) {
        let c0 = self.cycles;
        let mut total = 0u64;
        for ch in 0..8 {
            if mask & (1 << ch) == 0 {
                continue;
            }
            let r = self.dma[ch];
            let (dmap, bbad, bank) = (r[0], r[1], r[4]);
            let mut a = r[2] as u16 | (r[3] as u16) << 8;
            let mut cnt = r[5] as u32 | (r[6] as u32) << 8;
            if cnt == 0 {
                cnt = 0x10000
            }
            let step: i32 = match (dmap >> 3) & 3 {
                0 => 1,
                2 => -1,
                _ => 0,
            };
            let offs: &[u8] = match dmap & 7 {
                0 => &[0],
                1 => &[0, 1],
                2 | 6 => &[0, 0],
                3 | 7 => &[0, 0, 1, 1],
                4 => &[0, 1, 2, 3],
                _ => &[0, 1, 0, 1],
            };
            self.stats.dma_transfers += 1;
            self.stats.dma_bytes += cnt as u64;
            match bbad {
                0x18 | 0x19 => self.stats.dma_to_vram += cnt as u64,
                0x22 => self.stats.dma_to_cgram += cnt as u64,
                0x04 => self.stats.dma_to_oam += cnt as u64,
                _ => self.stats.dma_to_other += cnt as u64,
            }
            let to_a = dmap & 0x80 != 0;
            let mut i = 0usize;
            let n = cnt;
            while cnt > 0 {
                let b = 0x2100u32 | bbad.wrapping_add(offs[i % offs.len()]) as u32;
                if !to_a {
                    let v = self.read_a(bank, a);
                    let _ = self.write_inner(b, v);
                } else {
                    let v = self.read_inner(b).0;
                    if !(bank <= 0x3F && (0x2100..=0x21FF).contains(&a)) {
                        let _ = self.write_inner(((bank as u32) << 16) | a as u32, v);
                    }
                }
                a = (a as i32 + step) as u16;
                cnt -= 1;
                i += 1;
            }
            self.dma[ch][2] = a as u8;
            self.dma[ch][3] = (a >> 8) as u8;
            self.dma[ch][5] = 0;
            self.dma[ch][6] = 0;
            total += 8 + 8 * n as u64;
        }
        self.cycles = c0 + total + 8;
    }
}

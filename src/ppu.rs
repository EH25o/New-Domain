//! Estado da PPU do SNES: VRAM, CGRAM, OAM e registradores.
//!
//! Por enquanto só guarda o estado (o que o jogo escreve por DMA/registradores).
//! O renderizador entra depois e lê estas tabelas.

pub struct Ppu {
    pub vram: Vec<u8>,    // 64 KB
    pub cgram: [u8; 512], // 256 cores BGR555
    pub oam: [u8; 544],   // 512 bytes baixos + 32 da tabela alta
    pub regs: [u8; 0x40], // último valor escrito em $2100-$213F
    pub vmain: u8,
    pub vmadd: u16,
    vram_latch: u16,
    cgadd: u16,
    cg_latch: u8,
    cg_hi: bool,
    oam_byte: u16,
    oam_latch: u8,
    cg_read_hi: bool,
    // latch de contadores (OPHCT/OPVCT)
    pub ophct: u16,
    pub opvct: u16,
    ophct_hi: bool,
    opvct_hi: bool,
    pub vram_writes: u64,
    pub cgram_writes: u64,
    pub oam_writes: u64,
}

impl Default for Ppu {
    fn default() -> Self {
        Ppu::new()
    }
}

impl Ppu {
    pub fn new() -> Ppu {
        Ppu {
            vram: vec![0; 0x10000],
            cgram: [0; 512],
            oam: [0; 544],
            regs: [0; 0x40],
            vmain: 0,
            vmadd: 0,
            vram_latch: 0,
            cgadd: 0,
            cg_latch: 0,
            cg_hi: false,
            oam_byte: 0,
            oam_latch: 0,
            cg_read_hi: false,
            ophct: 0,
            opvct: 0,
            ophct_hi: false,
            opvct_hi: false,
            vram_writes: 0,
            cgram_writes: 0,
            oam_writes: 0,
        }
    }

    fn vram_step(&self) -> u16 {
        match self.vmain & 3 {
            0 => 1,
            1 => 32,
            _ => 128,
        }
    }
    fn vram_idx(&self) -> usize {
        ((self.vmadd as usize) & 0x7FFF) * 2
    }

    pub fn write(&mut self, reg: u8, v: u8) {
        if (reg as usize) < self.regs.len() {
            self.regs[reg as usize] = v
        }
        match reg {
            0x02 => self.oam_byte = (self.oam_byte & 0x200) | (v as u16) << 1,
            0x03 => self.oam_byte = (self.oam_byte & 0x1FE) | ((v as u16 & 1) << 9),
            0x04 => {
                let b = self.oam_byte as usize;
                if b < 512 {
                    if b & 1 == 0 {
                        self.oam_latch = v
                    } else {
                        self.oam[b - 1] = self.oam_latch;
                        self.oam[b] = v
                    }
                } else {
                    self.oam[512 + (b & 0x1F)] = v;
                }
                self.oam_writes += 1;
                self.oam_byte = (self.oam_byte + 1) & 0x3FF;
            }
            0x15 => self.vmain = v,
            0x16 => {
                self.vmadd = (self.vmadd & 0xFF00) | v as u16;
                self.prefetch()
            }
            0x17 => {
                self.vmadd = (self.vmadd & 0x00FF) | (v as u16) << 8;
                self.prefetch()
            }
            0x18 => {
                let i = self.vram_idx();
                self.vram[i] = v;
                self.vram_writes += 1;
                if self.vmain & 0x80 == 0 {
                    self.vmadd = self.vmadd.wrapping_add(self.vram_step())
                }
            }
            0x19 => {
                let i = self.vram_idx();
                self.vram[i + 1] = v;
                self.vram_writes += 1;
                if self.vmain & 0x80 != 0 {
                    self.vmadd = self.vmadd.wrapping_add(self.vram_step())
                }
            }
            0x21 => {
                self.cgadd = v as u16;
                self.cg_hi = false;
                self.cg_read_hi = false
            }
            0x22 => {
                if !self.cg_hi {
                    self.cg_latch = v
                } else {
                    let i = (self.cgadd as usize & 0xFF) * 2;
                    self.cgram[i] = self.cg_latch;
                    self.cgram[i + 1] = v & 0x7F;
                    self.cgadd = self.cgadd.wrapping_add(1);
                    self.cgram_writes += 1;
                }
                self.cg_hi = !self.cg_hi;
            }
            _ => {}
        }
    }

    fn prefetch(&mut self) {
        let i = self.vram_idx();
        self.vram_latch = self.vram[i] as u16 | (self.vram[i + 1] as u16) << 8;
    }

    /// Leitura de registrador PPU. `None` = barramento aberto.
    pub fn read(&mut self, reg: u8) -> Option<u8> {
        match reg {
            0x38 => {
                let b = self.oam_byte as usize;
                let v = if b < 512 {
                    self.oam[b]
                } else {
                    self.oam[512 + (b & 0x1F)]
                };
                self.oam_byte = (self.oam_byte + 1) & 0x3FF;
                Some(v)
            }
            0x39 | 0x3A => {
                let v = if reg == 0x39 {
                    self.vram_latch as u8
                } else {
                    (self.vram_latch >> 8) as u8
                };
                let inc_on_high = self.vmain & 0x80 != 0;
                if (reg == 0x3A) == inc_on_high {
                    self.prefetch();
                    self.vmadd = self.vmadd.wrapping_add(self.vram_step());
                }
                Some(v)
            }
            0x3B => {
                let i = (self.cgadd as usize & 0xFF) * 2;
                let v = self.cgram[i + self.cg_read_hi as usize];
                if self.cg_read_hi {
                    self.cgadd = self.cgadd.wrapping_add(1)
                }
                self.cg_read_hi = !self.cg_read_hi;
                Some(v)
            }
            0x3C => {
                self.ophct_hi = !self.ophct_hi;
                Some(if self.ophct_hi {
                    self.ophct as u8
                } else {
                    (self.ophct >> 8) as u8 & 1
                })
            }
            0x3D => {
                self.opvct_hi = !self.opvct_hi;
                Some(if self.opvct_hi {
                    self.opvct as u8
                } else {
                    (self.opvct >> 8) as u8 & 1
                })
            }
            0x3E => Some(0x01), // STAT77: versão do chip
            0x3F => {
                self.ophct_hi = false;
                self.opvct_hi = false;
                Some(0x03)
            } // STAT78: NTSC, versão
            0x34..=0x36 => Some(0), // resultado da multiplicação do Modo 7
            _ => None,
        }
    }

    pub fn latch_counters(&mut self, h: u16, v: u16) {
        self.ophct = h;
        self.opvct = v;
    }
}

//! Carregamento e mapeamento de ROM SNES LoROM.
//!
//! A ROM nunca é embutida no código: ela é lida do disco, da sua própria cópia.

use std::{fs, io, path::Path};

pub const LOROM_HEADER: usize = 0x7FC0;

pub struct Rom {
    pub data: Vec<u8>,
    pub had_copier_header: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Header {
    pub title: String,
    pub map_mode: u8,
    pub rom_type: u8,
    pub rom_size_kb: u32,
    pub sram_kb: u32,
    pub region: u8,
    pub license: u8,
    pub version: u8,
    pub complement: u16,
    pub checksum: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct Vectors {
    pub native_nmi: u16,
    pub native_irq: u16,
    pub native_cop: u16,
    pub native_brk: u16,
    pub native_abort: u16,
    pub emu_reset: u16,
}

impl Rom {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Rom> {
        Ok(Rom::from_bytes(fs::read(path)?))
    }

    /// Remove o "copier header" de 512 bytes, se existir (arquivos .smc antigos).
    pub fn from_bytes(mut data: Vec<u8>) -> Rom {
        let had = data.len() % 1024 == 512;
        if had {
            data.drain(..512);
        }
        Rom {
            data,
            had_copier_header: had,
        }
    }

    /// Endereço de barramento (banco:endereço) -> offset no arquivo (LoROM).
    /// `offset = (banco & 0x7F) * 0x8000 + (endereço - 0x8000)`.
    /// Retorna `None` para a metade baixa dos bancos (RAM/registradores) e para a WRAM (7E/7F).
    pub fn offset(&self, bank: u8, addr: u16) -> Option<usize> {
        if addr < 0x8000 || bank == 0x7E || bank == 0x7F || self.data.is_empty() {
            return None;
        }
        let off = (bank & 0x7F) as usize * 0x8000 + (addr as usize - 0x8000);
        Some(off % self.data.len())
    }

    pub fn read(&self, bank: u8, addr: u16) -> Option<u8> {
        self.offset(bank, addr).map(|o| self.data[o])
    }

    pub fn read_u16(&self, bank: u8, addr: u16) -> Option<u16> {
        Some(self.read(bank, addr)? as u16 | (self.read(bank, addr.wrapping_add(1))? as u16) << 8)
    }

    /// Soma de verificação simples (válida para ROMs de tamanho potência de 2, como esta).
    pub fn computed_checksum(&self) -> u16 {
        self.data
            .iter()
            .fold(0u16, |s, &b| s.wrapping_add(b as u16))
    }

    pub fn header(&self) -> Result<Header, String> {
        let h = self
            .data
            .get(LOROM_HEADER..LOROM_HEADER + 0x20)
            .ok_or("ROM menor que o cabeçalho LoROM")?;
        let comp = u16::from_le_bytes([h[0x1C], h[0x1D]]);
        let sum = u16::from_le_bytes([h[0x1E], h[0x1F]]);
        if comp ^ sum != 0xFFFF {
            return Err(format!(
                "cabeçalho inválido: complemento {comp:04X} e checksum {sum:04X} não batem"
            ));
        }
        if h[0x15] & 0x0F != 0 {
            return Err(format!("modo de mapeamento {:02X} não é LoROM", h[0x15]));
        }
        Ok(Header {
            title: String::from_utf8_lossy(&h[0..21]).trim_end().to_string(),
            map_mode: h[0x15],
            rom_type: h[0x16],
            rom_size_kb: 1u32 << h[0x17],
            sram_kb: if h[0x18] == 0 { 0 } else { 1u32 << h[0x18] },
            region: h[0x19],
            license: h[0x1A],
            version: h[0x1B],
            complement: comp,
            checksum: sum,
        })
    }

    pub fn vectors(&self) -> Option<Vectors> {
        let r = |a: u16| self.read_u16(0, a);
        Some(Vectors {
            native_cop: r(0xFFE4)?,
            native_brk: r(0xFFE6)?,
            native_abort: r(0xFFE8)?,
            native_nmi: r(0xFFEA)?,
            native_irq: r(0xFFEE)?,
            emu_reset: r(0xFFFC)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lorom_mapping() {
        let mut d = vec![0u8; 0x10_0000];
        d[0] = 0xAA;
        d[0x8000] = 0xBB;
        let rom = Rom::from_bytes(d);
        assert_eq!(rom.read(0x00, 0x8000), Some(0xAA));
        assert_eq!(rom.read(0x80, 0x8000), Some(0xAA)); // espelho
        assert_eq!(rom.read(0x01, 0x8000), Some(0xBB));
        assert_eq!(rom.read(0x00, 0x1234), None); // WRAM/registradores
        assert_eq!(rom.read(0x7E, 0x8000), None); // WRAM
    }

    #[test]
    fn strips_copier_header() {
        let rom = Rom::from_bytes(vec![0u8; 512 + 0x8000]);
        assert!(rom.had_copier_header);
        assert_eq!(rom.data.len(), 0x8000);
    }
}

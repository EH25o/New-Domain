//! Mapa de memória do Final Fight (USA) descoberto por engenharia reversa.
//!
//! Cada entrada tem um nível de confiança:
//! * `Confirmado`: validado no emulador (dumps da WRAM ou breakpoints).
//! * `Hipotese`: deduzido do código, ainda precisa de teste.
//!
//! Atualize este arquivo à medida que descobrir mais endereços: ele é a "especificação"
//! que a reescrita em Rust usa e que o teste frame a frame compara com o emulador.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    Confirmado,
    Hipotese,
}

pub struct Region {
    pub start: u32,
    pub len: u32,
    pub name: &'static str,
    pub conf: Confidence,
}

use Confidence::*;

/// Endereços de WRAM (baixos, `$0000-$1FFF`, espelhados em `$7E0000`).
pub const WRAM_REGIONS: &[Region] = &[
    Region { start: 0x0036, len: 1, name: "irq_state (indice da tabela de IRQ)", conf: Hipotese },
    Region { start: 0x0040, len: 0x90, name: "task_table (9 tarefas de 16 bytes em $40-$C0: +0 status, +1 espera, +2 SP salvo, +4 topo da pilha [confirmado: $1DFF+$40*n], +6 indice de estado)", conf: Hipotese },
    Region { start: 0x00C8, len: 1, name: "frame_sync_flag", conf: Hipotese },
    Region { start: 0x00C9, len: 1, name: "frame_counter", conf: Confirmado },
    Region { start: 0x00E8, len: 1, name: "INIDISP_shadow (copia de $2100)", conf: Confirmado },
    Region { start: 0x0108, len: 1, name: "NMITIMEN_shadow (copia de $4200)", conf: Confirmado },
    Region { start: 0x010A, len: 4, name: "rng_state (muda a cada frame)", conf: Hipotese },
    Region { start: 0x0148, len: 1, name: "oam_high_x_accum (bits extras de X)", conf: Hipotese },
    Region { start: 0x0180, len: 0x200, name: "oam_shadow (128 sprites x 4 bytes: x, y, tile, attr)", conf: Confirmado },
    Region { start: 0x0380, len: 0x20, name: "oam_high_table (bit 8 de X e tamanho)", conf: Confirmado },
    Region { start: 0x1E00, len: 0x200, name: "pilhas_das_tarefas (uma por tarefa; ruido nos dumps)", conf: Hipotese },
    Region { start: 0x00CE, len: 1, name: "game_running_gate (NMI so roda a logica se != 0)", conf: Hipotese },
    Region { start: 0x00D7, len: 1, name: "nmi_skip_logic (copia de $0104)", conf: Hipotese },
];

#[allow(dead_code)]
pub const FRAME_COUNTER: usize = 0x00C9;
pub const OAM_SHADOW: usize = 0x0180;
pub const OAM_HIGH: usize = 0x0380;
pub const OAM_ENTRIES: usize = 128;

/// Nome legível para um endereço (WRAM baixa ou registrador de hardware), se conhecido.
pub fn name(addr: u32) -> Option<String> {
    let a = addr & 0xFFFF;
    if a < 0x2000 {
        // Prefere a região mais específica (menor) quando há sobreposição.
        return WRAM_REGIONS
            .iter()
            .filter(|r| a >= r.start && a < r.start + r.len)
            .min_by_key(|r| r.len)
            .map(|r| {
                let tag = if r.conf == Confirmado { "" } else { "?" };
                if r.len > 1 {
                    format!(
                        "{}+{:#X}{}",
                        r.name.split(' ').next().unwrap_or(r.name),
                        a - r.start,
                        tag
                    )
                } else {
                    format!("{}{}", r.name.split(' ').next().unwrap_or(r.name), tag)
                }
            });
    }
    let hw = match a {
        0x2100 => "INIDISP",
        0x2101 => "OBSEL",
        0x2102 => "OAMADDL",
        0x2103 => "OAMADDH",
        0x2104 => "OAMDATA",
        0x2105 => "BGMODE",
        0x2106 => "MOSAIC",
        0x2107..=0x210A => "BGnSC",
        0x210B..=0x210C => "BGnNBA",
        0x210D..=0x2114 => "BGnHOFS/VOFS",
        0x2115 => "VMAIN",
        0x2116 => "VMADDL",
        0x2117 => "VMADDH",
        0x2118 => "VMDATAL",
        0x2119 => "VMDATAH",
        0x2121 => "CGADD",
        0x2122 => "CGDATA",
        0x212C => "TM",
        0x212D => "TS",
        0x2130..=0x2132 => "CGWSEL/CGADSUB/COLDATA",
        0x2140..=0x2143 => "APUIO",
        0x2180 => "WMDATA",
        0x2181..=0x2183 => "WMADD",
        0x4016 | 0x4017 => "JOYSER",
        0x4200 => "NMITIMEN",
        0x4201 => "WRIO",
        0x4202 | 0x4203 => "WRMPY",
        0x4204..=0x4206 => "WRDIV",
        0x4207..=0x420A => "HTIME/VTIME",
        0x420B => "MDMAEN",
        0x420C => "HDMAEN",
        0x420D => "MEMSEL",
        0x4210 => "RDNMI",
        0x4211 => "TIMEUP",
        0x4212 => "HVBJOY",
        0x4214..=0x4217 => "RDDIV/RDMPY",
        0x4218..=0x421F => "JOYxL/H",
        0x4300..=0x437F => "DMA/HDMA canal",
        _ => return None,
    };
    Some(hw.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sprite {
    pub index: usize,
    pub x: u16, // 9 bits
    pub y: u8,
    pub tile: u8,
    pub attr: u8,
    pub large: bool,
}

impl Sprite {
    /// Sprite fora da tela (o jogo limpa o OAM com Y = $E0).
    pub fn hidden(&self) -> bool {
        self.y >= 0xE0
    }
}

/// Decodifica o OAM a partir de um dump da WRAM (128 KB ou só os primeiros 8 KB).
pub fn decode_oam(wram: &[u8]) -> Vec<Sprite> {
    if wram.len() < OAM_HIGH + 0x20 {
        return Vec::new();
    }
    (0..OAM_ENTRIES)
        .map(|i| {
            let o = OAM_SHADOW + i * 4;
            let hi = wram[OAM_HIGH + i / 4] >> ((i % 4) * 2);
            Sprite {
                index: i,
                x: wram[o] as u16 | ((hi as u16 & 1) << 8),
                y: wram[o + 1],
                tile: wram[o + 2],
                attr: wram[o + 3],
                large: hi & 2 != 0,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(name(0x00C9).as_deref(), Some("frame_counter"));
        assert_eq!(name(0x4200).as_deref(), Some("NMITIMEN"));
        assert_eq!(name(0x0181).as_deref(), Some("oam_shadow+0x1"));
        assert!(name(0x1000).is_none());
    }

    #[test]
    fn oam_high_bits() {
        let mut w = vec![0u8; 0x20000];
        w[OAM_SHADOW] = 0x10;
        w[OAM_SHADOW + 1] = 0x20;
        w[OAM_HIGH] = 0b0000_0011; // sprite 0: X bit 8 e tamanho grande
        let s = decode_oam(&w);
        assert_eq!((s[0].x, s[0].y, s[0].large), (0x110, 0x20, true));
    }
}

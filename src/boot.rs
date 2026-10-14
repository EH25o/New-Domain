//! Máquina completa (CPU + barramento + PPU/APU parciais) para rodar a ROM e comparar estados.
//!
//! Serve de **referência semântica** para a recompilação: o código gerado depois tem que
//! deixar a WRAM igual à deste interpretador, quadro a quadro.
//!
//! Limites atuais (documentados porque aparecem como travamentos):
//! * a PPU só guarda estado, não desenha;
//! * a APU só implementa o protocolo de upload da ROM de boot (IPL); não há SPC700 nem som.
//!   Por isso o jogo trava quando pede para recarregar o driver de som (`$BBAA` na porta 0).

use crate::bus::Bus;
use crate::cpu::{Cpu, I};
use crate::rom::Rom;

const LINE_CYCLES: u64 = 1364; // ciclos mestres por scanline
const LINES: u16 = 262;
const VBLANK_LINE: u16 = 225;

pub struct Emu {
    pub bus: Bus,
    pub cpu: Cpu,
    pub frame: u32,
    next_line: u64,
}

impl Emu {
    pub fn new(rom: Rom) -> Emu {
        let mut bus = Bus::new(rom);
        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        let next_line = bus.cycles;
        Emu {
            bus,
            cpu,
            frame: 0,
            next_line,
        }
    }

    /// Roda um quadro de vídeo (262 linhas), disparando NMI no início do vblank e IRQ por linha.
    pub fn run_frame(&mut self) {
        for line in 0..LINES {
            self.bus.vcount = line;
            let line_start = self.next_line;
            self.next_line += LINE_CYCLES;

            if line == VBLANK_LINE {
                self.bus.nmi_flag = true;
                self.bus.latch_auto_joypad();
                self.bus.autojoy_busy = true;
                if self.bus.nmitimen & 0x80 != 0 {
                    self.cpu.waiting = false;
                    self.cpu.nmi(&mut self.bus);
                }
            }
            if line == VBLANK_LINE + 4 {
                self.bus.autojoy_busy = false;
            }
            let fire = match (self.bus.nmitimen >> 4) & 3 {
                1 => true,
                2 | 3 => line == self.bus.vtime,
                _ => false,
            };
            if fire {
                self.bus.irq_flag = true;
                if self.cpu.p & I == 0 {
                    self.cpu.waiting = false;
                    self.cpu.irq(&mut self.bus);
                }
            }

            while self.bus.cycles < self.next_line && !self.cpu.stopped {
                self.bus.hclk = self
                    .bus
                    .cycles
                    .saturating_sub(line_start)
                    .min(LINE_CYCLES - 1) as u32;
                self.cpu.step(&mut self.bus);
                self.bus.apu.sync(self.bus.cycles);
            }
        }
        self.frame += 1;
    }
}

/// Comando `boot`: roda N quadros e imprime um resumo.
pub fn run_command(rom: Rom, frames: u32, dump: Option<&str>) -> Result<(), String> {
    let mut emu = Emu::new(rom);
    let (mut last_pc, mut stuck) = ((0u8, 0u16), 0u32);
    for _ in 0..frames {
        emu.run_frame();
        let pc = (emu.cpu.pb, emu.cpu.pc);
        if emu.frame == 1 || emu.frame % 60 == 0 {
            println!("quadro {:4}  $C9={:02X}  $E8={:02X}  S={:04X}  PC={:02X}:{:04X}  instr={}  apu={:?}",
                emu.frame, emu.bus.wram[0xC9], emu.bus.wram[0xE8], emu.cpu.s, pc.0, pc.1, emu.cpu.instr_count, emu.bus.apu.state);
        }
        stuck = if pc == last_pc { stuck + 1 } else { 0 };
        last_pc = pc;
        if stuck >= 120 {
            println!("!! o PC ficou em {:02X}:{:04X} por 120 quadros", pc.0, pc.1);
            println!(
                "SPC PC={:04X}, portas CPU->{:02X?}, SPC->{:02X?}",
                emu.bus.apu.spc.pc, emu.bus.apu.spc.cpu_to_spc, emu.bus.apu.spc.spc_to_cpu
            );
            for (address, opcode, cycles) in &emu.bus.apu.trace {
                println!("  SPC ${address:04X}: ${opcode:02X} ({cycles} ciclos)");
            }
            break;
        }
    }
    let (b, c) = (&emu.bus, &emu.cpu);
    println!("\n== Resumo ({} quadros) ==", emu.frame);
    println!(
        "instruções: {}   ciclos mestres: {}   opcodes decimais: {}",
        c.instr_count, b.cycles, c.decimal_ops
    );
    println!(
        "APU: {:?}, {} blocos enviados, entrada {:?}, reentradas IPL {}",
        b.apu.state,
        b.apu.uploads.len(),
        b.apu.entry,
        b.apu.ipl_reentries
    );
    println!(
        "DMA: {} transferências, {} bytes (VRAM {}, CGRAM {}, OAM {}, outros {})",
        b.stats.dma_transfers,
        b.stats.dma_bytes,
        b.stats.dma_to_vram,
        b.stats.dma_to_cgram,
        b.stats.dma_to_oam,
        b.stats.dma_to_other
    );
    let mut r: Vec<_> = b.stats.unknown_reads.iter().collect();
    r.sort_by_key(|x| std::cmp::Reverse(*x.1));
    let mut w: Vec<_> = b.stats.unknown_writes.iter().collect();
    w.sort_by_key(|x| std::cmp::Reverse(*x.1));
    println!(
        "leituras de registradores não tratados: {:?}",
        r.iter()
            .take(6)
            .map(|(a, n)| format!("{a:04X}x{n}"))
            .collect::<Vec<_>>()
    );
    println!(
        "escritas em registradores não tratados: {:?}",
        w.iter()
            .take(6)
            .map(|(a, n)| format!("{a:04X}x{n}"))
            .collect::<Vec<_>>()
    );
    if let Some(f) = dump {
        std::fs::write(f, &b.wram).map_err(|e| format!("{f}: {e}"))?;
        println!("WRAM (128 KB) gravada em {f}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regressão: precisa da ROM. Rode com `FF_ROM=caminho/da/rom.sfc cargo test`.
    /// Os valores esperados vêm de dumps reais da WRAM tirados no Mesen2.
    #[test]
    fn boots_and_matches_real_wram_structure() {
        let Ok(path) = std::env::var("FF_ROM") else {
            eprintln!("FF_ROM não definido: teste ignorado");
            return;
        };
        let mut emu = Emu::new(Rom::load(path).expect("ROM"));
        for _ in 0..160 {
            emu.run_frame();
        }
        let w = &emu.bus.wram;
        assert_eq!(w[0xE8], 0x0F, "cópia de $2100 (tela ligada)");
        assert_eq!(w[0x108], 0x31, "cópia de $4200");
        assert_eq!(w[0xCE], 0x01, "jogo em execução");
        assert!(
            w[0xC9] >= 100,
            "o NMI tem que estar contando quadros, $C9 = {:02X}",
            w[0xC9]
        );
        // Topos de pilha das 8 tarefas: idênticos aos do dump real do Mesen2.
        let tops: Vec<u16> = (0..8)
            .map(|k| u16::from_le_bytes([w[0x44 + 16 * k], w[0x45 + 16 * k]]))
            .collect();
        assert_eq!(
            tops,
            [0x1DFF, 0x1E3F, 0x1E7F, 0x1EBF, 0x1EFF, 0x1F3F, 0x1F7F, 0x1FBF]
        );
    }

    #[test]
    fn boots_through_900_frames_with_the_uploaded_spc_driver() {
        let Ok(path) = std::env::var("FF_ROM") else {
            eprintln!("FF_ROM não definido: teste ignorado");
            return;
        };
        let mut emu = Emu::new(Rom::load(path).expect("ROM"));
        for _ in 0..190 {
            emu.run_frame();
        }
        let c9_at_190 = emu.bus.wram[0xC9];
        for _ in 190..900 {
            emu.run_frame();
        }

        assert_eq!(emu.frame, 900);
        assert_eq!(emu.bus.apu.state, crate::apu::State::Running);
        assert!(
            emu.bus.wram[0xC9] != c9_at_190,
            "$C9 stopped advancing after frame 190: still ${c9_at_190:02X}"
        );
        assert!(!emu.cpu.stopped, "65816 entered STP before frame 900");
    }
}

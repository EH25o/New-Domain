//! Descoberta de código por varredura recursiva (estágio 1 da recompilação estática).
//!
//! Parte dos vetores (RESET, NMI, IRQ) e segue desvios, `JSR`/`JSL` e `JMP`, acompanhando os flags
//! M/X de cada caminho. O resultado separa código de dados e lista o que a análise estática NÃO
//! consegue resolver sozinha (saltos indiretos por tabela), que é o trabalho manual restante.
//!
//! Limitações conhecidas (não são bugs, são o custo da análise estática):
//! * assume que uma sub-rotina devolve M/X como recebeu; rotinas que os trocam e retornam sem
//!   `PLP` podem decodificar o código seguinte errado (aparece em "decodificações ambíguas");
//! * não resolve `RTS`/`RTL` usados como salto computado nem dados embutidos depois de um `JSR`;
//! * tabelas de salto só são seguidas se declaradas à mão ou, com `--speculative`, por heurística.

use crate::disasm::{self, Flags, Insn};
use crate::optable::Mode;
use crate::rom::Rom;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

/// Posição canônica na ROM: (banco 0..N, endereço $8000-$FFFF). Espelhos viram o mesmo `Loc`.
pub type Loc = (u8, u16);

#[derive(Default, Clone)]
pub struct Options {
    pub speculative: bool,
    /// Tabelas de salto declaradas pelo usuário: (banco, endereço da tabela, nº de entradas).
    pub manual_tables: Vec<(u8, u16, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Unresolved,
    Manual,
    Heuristic,
}

#[derive(Clone, Debug)]
pub struct IndirectSite {
    pub at: Loc,
    pub mnemonic: &'static str,
    pub operand: String,
    pub table: Option<Loc>,
    pub targets: Vec<u16>,
    pub resolution: Resolution,
}

pub struct Node {
    pub insn: Insn,
    pub flags: BTreeSet<Flags>,
}

#[derive(Default)]
pub struct Analysis {
    pub nodes: BTreeMap<Loc, Node>,
    /// Entradas de função (alvos de JSR/JSL e vetores) e os estados M/X com que foram chamadas.
    pub functions: BTreeMap<Loc, BTreeSet<Flags>>,
    pub indirect: Vec<IndirectSite>,
    /// Endereços decodificados com tamanhos diferentes conforme os flags M/X.
    pub ambiguous: BTreeSet<Loc>,
    /// Destinos fora da ROM (ex.: código que roda na RAM): destino -> origem.
    pub undecodable: BTreeMap<(u8, u16), Loc>,
}

struct Ex<'a> {
    rom: &'a Rom,
    a: Analysis,
    seen: HashSet<(Loc, Flags)>,
    q: VecDeque<(Loc, Flags)>,
}

impl<'a> Ex<'a> {
    fn canon(&self, bank: u8, addr: u16) -> Option<Loc> {
        self.rom
            .offset(bank, addr)
            .map(|o| ((o >> 15) as u8, 0x8000 | (o & 0x7FFF) as u16))
    }

    fn push(&mut self, bank: u8, addr: u16, f: Flags, origin: Loc) {
        match self.canon(bank, addr) {
            Some(l) => self.q.push_back((l, f)),
            None => {
                self.a.undecodable.entry((bank, addr)).or_insert(origin);
            }
        }
    }

    fn func(&mut self, bank: u8, addr: u16, f: Flags, origin: Loc) {
        if let Some(l) = self.canon(bank, addr) {
            self.a.functions.entry(l).or_default().insert(f);
        }
        self.push(bank, addr, f, origin);
    }

    fn run(&mut self) {
        while let Some((loc, f)) = self.q.pop_front() {
            if self.seen.insert((loc, f)) {
                self.step(loc, f);
            }
        }
    }

    fn step(&mut self, loc: Loc, f: Flags) {
        let (bank, addr) = loc;
        let Some(insn) = disasm::decode(self.rom, bank, addr, f, false) else {
            return;
        };
        let len = insn.bytes.len();
        match self.a.nodes.get_mut(&loc) {
            Some(n) => {
                if n.insn.bytes.len() != len {
                    self.a.ambiguous.insert(loc);
                }
                n.flags.insert(f);
            }
            None => {
                let mut s = BTreeSet::new();
                s.insert(f);
                self.a.nodes.insert(
                    loc,
                    Node {
                        insn: insn.clone(),
                        flags: s,
                    },
                );
            }
        }

        let mut nf = f;
        disasm::step_flags(&mut nf, &insn);
        let next = addr.wrapping_add(len as u16);
        let v = insn.value;
        let rel = || match insn.mode {
            Mode::Rel8 => next.wrapping_add(v as u8 as i8 as i16 as u16),
            _ => next.wrapping_add(v as u16),
        };

        match insn.mnemonic {
            "BPL" | "BMI" | "BVC" | "BVS" | "BCC" | "BCS" | "BNE" | "BEQ" => {
                self.push(bank, rel(), nf, loc);
                self.push(bank, next, nf, loc);
            }
            "BRA" | "BRL" => self.push(bank, rel(), nf, loc),
            "JMP" => match insn.mode {
                Mode::Abs => self.push(bank, v as u16, nf, loc),
                _ => self.indirect(loc, &insn),
            },
            "JML" => match insn.mode {
                Mode::Long => self.push((v >> 16) as u8, v as u16, nf, loc),
                _ => self.indirect(loc, &insn),
            },
            "JSR" => {
                match insn.mode {
                    Mode::Abs => self.func(bank, v as u16, nf, loc),
                    _ => self.indirect(loc, &insn),
                }
                self.push(bank, next, nf, loc);
            }
            "JSL" => {
                self.func((v >> 16) as u8, v as u16, nf, loc);
                self.push(bank, next, nf, loc);
            }
            "RTS" | "RTL" | "RTI" | "BRK" | "COP" | "STP" => {}
            _ => self.push(bank, next, nf, loc),
        }
    }

    fn indirect(&mut self, loc: Loc, insn: &Insn) {
        if self.a.indirect.iter().any(|s| s.at == loc) {
            return;
        }
        // Só `(a,X)` lê a tabela no banco do programa; `(a)` e `[a]` leem da RAM (banco 0).
        let table = if insn.mode == Mode::AbsIndX {
            self.canon(loc.0, insn.value as u16)
        } else {
            None
        };
        self.a.indirect.push(IndirectSite {
            at: loc,
            mnemonic: insn.mnemonic,
            operand: insn.operand.clone(),
            table,
            targets: Vec::new(),
            resolution: Resolution::Unresolved,
        });
    }
}

/// Lê entradas de 16 bits de uma tabela enquanto parecerem endereços de código válidos.
/// Para antes de entrar no código que a própria tabela referencia: se a entrada 0 aponta para
/// `tabela + 8`, a tabela tem exatamente 4 entradas.
fn read_table_heuristic(rom: &Rom, bank: u8, addr: u16, limit: usize) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    for i in 0..limit {
        let pos = addr.wrapping_add(2 * i as u16);
        if out.iter().any(|&t| t > addr && pos >= t) {
            break;
        }
        let Some(e) = rom.read_u16(bank, pos) else {
            break;
        };
        if e < 0x8000 || rom.read(bank, e).is_none_or(|b| b == 0x00) {
            break;
        }
        out.push(e);
    }
    out
}

/// Analisa a partir de pontos de entrada explícitos (usado nos testes).
pub fn analyze_from(rom: &Rom, seeds: &[(u8, u16)], opts: &Options) -> Analysis {
    let mut ex = Ex {
        rom,
        a: Analysis::default(),
        seen: HashSet::new(),
        q: VecDeque::new(),
    };
    let start = Flags::default();
    for &(b, a) in seeds {
        ex.func(b, a, start, (b, a));
    }
    ex.run();

    let manual: BTreeMap<Loc, usize> = opts
        .manual_tables
        .iter()
        .filter_map(|&(b, a, n)| ex.canon(b, a).map(|l| (l, n)))
        .collect();

    loop {
        let mut progressed = false;
        for idx in 0..ex.a.indirect.len() {
            let site = ex.a.indirect[idx].clone();
            if site.resolution != Resolution::Unresolved {
                continue;
            }
            let Some(table) = site.table else { continue };
            let (targets, how) = if let Some(&n) = manual.get(&table) {
                let t: Vec<u16> = (0..n)
                    .filter_map(|i| rom.read_u16(table.0, table.1.wrapping_add(2 * i as u16)))
                    .filter(|&e| e >= 0x8000)
                    .collect();
                (t, Resolution::Manual)
            } else if opts.speculative {
                (
                    read_table_heuristic(rom, table.0, table.1, 64),
                    Resolution::Heuristic,
                )
            } else {
                continue;
            };
            let flags: Vec<Flags> =
                ex.a.nodes
                    .get(&site.at)
                    .map(|n| n.flags.iter().copied().collect())
                    .unwrap_or_default();
            for &t in &targets {
                for &f in &flags {
                    if site.mnemonic == "JSR" {
                        ex.func(site.at.0, t, f, site.at)
                    } else {
                        ex.push(site.at.0, t, f, site.at)
                    }
                }
            }
            ex.a.indirect[idx].targets = targets;
            ex.a.indirect[idx].resolution = how;
            progressed = true;
        }
        if !progressed {
            break;
        }
        ex.run();
    }
    ex.a
}

/// Analisa a ROM a partir dos vetores RESET, NMI e IRQ.
pub fn analyze(rom: &Rom, opts: &Options) -> Analysis {
    let mut seeds = Vec::new();
    if let Some(v) = rom.vectors() {
        seeds.extend([(0u8, v.emu_reset), (0, v.native_nmi), (0, v.native_irq)]);
    }
    analyze_from(rom, &seeds, opts)
}

impl Analysis {
    pub fn code_bytes(&self) -> usize {
        self.nodes.values().map(|n| n.insn.bytes.len()).sum()
    }

    /// Instruções cujos bytes se sobrepõem a outra instrução (sinal de decodificação errada).
    pub fn overlaps(&self) -> usize {
        let mut n = 0;
        let mut prev: Option<(u8, u32)> = None; // banco, fim da instrução anterior
        for (&(b, a), node) in &self.nodes {
            if let Some((pb, end)) = prev {
                if pb == b && (a as u32) < end {
                    n += 1;
                }
            }
            prev = Some((b, a as u32 + node.insn.bytes.len() as u32));
        }
        n
    }

    pub fn per_bank(&self) -> BTreeMap<u8, usize> {
        let mut m = BTreeMap::new();
        for (&(b, _), n) in &self.nodes {
            *m.entry(b).or_default() += n.insn.bytes.len();
        }
        m
    }
}

pub fn parse_tables(text: &str) -> Result<Vec<(u8, u16, usize)>, String> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let err = || format!("linha {}: use 'BB:AAAA N' (ex.: 02:81F6 3)", i + 1);
        let mut it = line.split_whitespace();
        let (addr, n) = (it.next().ok_or_else(err)?, it.next().ok_or_else(err)?);
        let (b, a) = addr
            .trim_start_matches('$')
            .split_once(':')
            .ok_or_else(err)?;
        out.push((
            u8::from_str_radix(b, 16).map_err(|_| err())?,
            u16::from_str_radix(a, 16).map_err(|_| err())?,
            n.parse().map_err(|_| err())?,
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom_with(parts: &[(usize, &[u8])]) -> Rom {
        let mut d = vec![0u8; 0x8000];
        for (off, b) in parts {
            d[*off..*off + b.len()].copy_from_slice(b);
        }
        Rom::from_bytes(d)
    }

    #[test]
    fn follows_calls_and_flags() {
        let rom = rom_with(&[
            (0x0000, &[0x20, 0x10, 0x80, 0x80, 0xFE]), // JSR $8010 / BRA $8003
            (0x0010, &[0xC2, 0x20, 0xA9, 0x34, 0x12, 0x60]), // REP #$20 / LDA #$1234 / RTS
        ]);
        let a = analyze_from(&rom, &[(0, 0x8000)], &Options::default());
        assert_eq!(a.nodes.len(), 5);
        assert!(a.functions.contains_key(&(0, 0x8010)));
        assert_eq!(a.nodes[&(0, 0x8012)].insn.bytes.len(), 3); // LDA de 16 bits
        assert_eq!(a.code_bytes(), 3 + 2 + 2 + 3 + 1);
        assert!(a.ambiguous.is_empty() && a.overlaps() == 0);
    }

    fn table_rom() -> Rom {
        rom_with(&[
            (0x0000, &[0xA2, 0x00, 0xFC, 0x0B, 0x80, 0x60]), // LDX #0 / JSR ($800B,X) / RTS
            (0x000B, &[0x10, 0x80, 0x20, 0x80, 0x00, 0x00]), // tabela: $8010, $8020, fim
            (0x0010, &[0x60]),
            (0x0020, &[0x60]),
        ])
    }

    #[test]
    fn indirect_unresolved_by_default() {
        let a = analyze_from(&table_rom(), &[(0, 0x8000)], &Options::default());
        assert_eq!(a.indirect.len(), 1);
        assert_eq!(a.indirect[0].resolution, Resolution::Unresolved);
        assert!(!a.functions.contains_key(&(0, 0x8010)));
    }

    #[test]
    fn indirect_heuristic_and_manual() {
        let h = analyze_from(
            &table_rom(),
            &[(0, 0x8000)],
            &Options {
                speculative: true,
                ..Default::default()
            },
        );
        assert_eq!(h.indirect[0].targets, vec![0x8010, 0x8020]);
        assert!(h.functions.contains_key(&(0, 0x8010)) && h.functions.contains_key(&(0, 0x8020)));

        let m = analyze_from(
            &table_rom(),
            &[(0, 0x8000)],
            &Options {
                manual_tables: vec![(0, 0x800B, 1)],
                ..Default::default()
            },
        );
        assert_eq!(m.indirect[0].resolution, Resolution::Manual);
        assert!(m.functions.contains_key(&(0, 0x8010)) && !m.functions.contains_key(&(0, 0x8020)));
    }

    #[test]
    fn heuristic_stops_before_own_target() {
        // Tabela de 2 entradas colada no código que ela mesma referencia: $800B -> $800F e $8011.
        let rom = rom_with(&[
            (0x0000, &[0xA2, 0x00, 0xFC, 0x0B, 0x80, 0x60]),
            (0x000B, &[0x0F, 0x80, 0x11, 0x80]),
            (0x000F, &[0xAD, 0xEA, 0x60, 0x60]), // código que parece uma 3ª entrada ($EAAD)
        ]);
        let a = analyze_from(
            &rom,
            &[(0, 0x8000)],
            &Options {
                speculative: true,
                ..Default::default()
            },
        );
        assert_eq!(a.indirect[0].targets, vec![0x800F, 0x8011]);
    }

    #[test]
    fn parses_table_file() {
        let t = parse_tables("# comentario\n02:81F6 3   # tres entradas\n\n$00:8804 4\n").unwrap();
        assert_eq!(t, vec![(2, 0x81F6, 3), (0, 0x8804, 4)]);
        assert!(parse_tables("lixo").is_err());
    }
}

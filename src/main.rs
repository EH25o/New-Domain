#![allow(dead_code)] // projeto em construção: várias peças ainda não são usadas pelo CLI
mod apu;
mod boot;
mod bus;
mod cfg;
mod cpu;
#[cfg(test)]
mod cpu_sst;
mod disasm;
mod dsp;
mod optable;
mod ppu;
mod rom;
mod spc700;
#[cfg(test)]
mod spc700_sst;
mod wram;

use disasm::Flags;
use rom::Rom;
use std::{env, fs, process};

const HELP: &str = "\
ffrs - ferramentas de engenharia reversa do Final Fight (SNES)

USO:
  ffrs info    <rom.sfc>                        cabeçalho, checksum e vetores
  ffrs disasm  <rom.sfc> <BB:AAAA> [N] [opções] desmonta N instruções (padrão 40)
        --m16 / --x16   começa com acumulador / índices de 16 bits
        --fn            para no primeiro RTS/RTL/RTI
        --dp0           anota acessos de direct page assumindo D = 0
  ffrs wramdiff <a.bin> <b.bin> [A-B]           compara dois dumps da WRAM (hex, faixa opcional)
  ffrs oam     <wram.bin> [--all]               decodifica os sprites do OAM shadow ($0180)
  ffrs map                                      lista o mapa de memória conhecido
  ffrs cfg     <rom.sfc> [opções]               descobre todo o código alcançável (recompilação, estágio 1)
        --tables ARQ    tabelas de salto declaradas à mão (linhas 'BB:AAAA N')
        --speculative   segue tabelas de salto por heurística (pode incluir lixo)
        --csv DIR       grava functions.csv e indirect.csv
        --ghidra ARQ    gera um script de Ghidra que desmonta cada função com o M/X certo (não testado)
  ffrs boot    <rom.sfc> [quadros] [--dump ARQ] roda a ROM no interpretador (padrão 600 quadros)

Exemplos:
  ffrs disasm ff.sfc 00:8000 30
  ffrs disasm ff.sfc 80:832F --fn --dp0
  ffrs wramdiff wram_parado.bin wram_direita.bin 0000-1FFF
";

fn parse_hex(s: &str) -> Result<u32, String> {
    u32::from_str_radix(s.trim_start_matches('$').trim_start_matches("0x"), 16)
        .map_err(|_| format!("hex inválido: {s}"))
}

fn parse_addr(s: &str) -> Result<(u8, u16), String> {
    let s = s.replace(['$', '_'], "");
    let (b, a) = match s.split_once(':') {
        Some((b, a)) => (parse_hex(b)?, parse_hex(a)?),
        None => {
            let v = parse_hex(&s)?;
            (v >> 16, v & 0xFFFF)
        }
    };
    if b > 0xFF || a > 0xFFFF {
        return Err(format!("endereço fora do limite: {s}"));
    }
    Ok((b as u8, a as u16))
}

fn load_rom(path: &str) -> Result<Rom, String> {
    Rom::load(path).map_err(|e| format!("não consegui abrir '{path}': {e}"))
}

fn cmd_info(path: &str) -> Result<(), String> {
    let rom = load_rom(path)?;
    let h = rom.header()?;
    println!("Título     : {}", h.title);
    println!(
        "Tamanho    : {} KB ({} bytes){}",
        h.rom_size_kb,
        rom.data.len(),
        if rom.had_copier_header {
            "  [copier header removido]"
        } else {
            ""
        }
    );
    println!(
        "Mapeamento : {:02X} (LoROM), tipo {:02X}, SRAM {} KB, região {:02X}, versão {}",
        h.map_mode, h.rom_type, h.sram_kb, h.region, h.version
    );
    let calc = rom.computed_checksum();
    println!(
        "Checksum   : {:04X} no cabeçalho, {:04X} calculado -> {}",
        h.checksum,
        calc,
        if calc == h.checksum {
            "OK"
        } else {
            "DIFERENTE"
        }
    );
    if let Some(v) = rom.vectors() {
        println!("Vetores    : RESET {:04X}  NMI {:04X}  IRQ {:04X}  (COP/BRK/ABORT {:04X}/{:04X}/{:04X})",
            v.emu_reset, v.native_nmi, v.native_irq, v.native_cop, v.native_brk, v.native_abort);
    }
    Ok(())
}

fn cmd_disasm(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("faltou o caminho da ROM")?;
    let (bank, addr) = parse_addr(args.get(1).ok_or("faltou o endereço (ex.: 00:8000)")?)?;
    let mut count = 40usize;
    let mut flags = Flags::default();
    let (mut stop, mut dp0) = (false, false);
    for a in &args[2..] {
        match a.as_str() {
            "--m16" => flags.m8 = false,
            "--x16" => flags.x8 = false,
            "--fn" => {
                stop = true;
                count = 100_000
            }
            "--dp0" => dp0 = true,
            n => {
                count = n
                    .parse()
                    .map_err(|_| format!("argumento desconhecido: {n}"))?
            }
        }
    }
    let rom = load_rom(path)?;
    let out = disasm::disassemble(&rom, bank, addr, count, flags, dp0, stop);
    if out.is_empty() {
        return Err(format!("{bank:02X}:{addr:04X} não está mapeado na ROM (use a metade alta do banco, $8000-$FFFF)"));
    }
    for (i, f) in out {
        let hex: Vec<String> = i.bytes.iter().map(|b| format!("{b:02X}")).collect();
        let c = i
            .comment
            .as_ref()
            .map(|c| format!("  ; {c}"))
            .unwrap_or_default();
        println!(
            "{:02X}:{:04X}  {:<11} {:<20}{}  [{}{}]",
            i.bank,
            i.addr,
            hex.join(" "),
            i.text(),
            c,
            if f.m8 { "M8" } else { "M16" },
            if f.x8 { " X8" } else { " X16" }
        );
    }
    Ok(())
}

fn cmd_wramdiff(args: &[String]) -> Result<(), String> {
    let (pa, pb) = (
        args.first().ok_or("faltou o dump A")?,
        args.get(1).ok_or("faltou o dump B")?,
    );
    let a = fs::read(pa).map_err(|e| format!("{pa}: {e}"))?;
    let b = fs::read(pb).map_err(|e| format!("{pb}: {e}"))?;
    if a.len() != b.len() {
        return Err(format!(
            "tamanhos diferentes: {} e {} bytes",
            a.len(),
            b.len()
        ));
    }
    let (lo, hi) = match args.get(2) {
        Some(r) => {
            let (l, h) = r
                .split_once('-')
                .ok_or("faixa deve ser A-B em hex, ex.: 0000-1FFF")?;
            (
                parse_hex(l)? as usize,
                (parse_hex(h)? as usize).min(a.len() - 1),
            )
        }
        None => (0, a.len() - 1),
    };
    let mut pages = std::collections::BTreeMap::<usize, usize>::new();
    let mut total = 0;
    for i in lo..=hi {
        if a[i] != b[i] {
            total += 1;
            *pages.entry(i >> 8).or_default() += 1;
            let n = wram::name(i as u32)
                .map(|n| format!("  ; {n}"))
                .unwrap_or_default();
            println!("{i:05X}: {:02X} -> {:02X}{n}", a[i], b[i]);
        }
    }
    println!("\n{total} bytes diferentes. Páginas de 256 bytes alteradas (página: bytes):");
    println!(
        "{}",
        pages
            .iter()
            .map(|(p, n)| format!("{p:02X}:{n}"))
            .collect::<Vec<_>>()
            .join("  ")
    );
    Ok(())
}

fn cmd_oam(args: &[String]) -> Result<(), String> {
    let p = args.first().ok_or("faltou o dump da WRAM")?;
    let w = fs::read(p).map_err(|e| format!("{p}: {e}"))?;
    let all = args.iter().any(|a| a == "--all");
    let sprites = wram::decode_oam(&w);
    if sprites.is_empty() {
        return Err("dump pequeno demais para conter o OAM".into());
    }
    let mut shown = 0;
    for s in &sprites {
        if s.hidden() && !all {
            continue;
        }
        shown += 1;
        println!(
            "#{:03}  x={:3} y={:3} tile={:02X} attr={:02X} {}",
            s.index,
            s.x,
            s.y,
            s.tile,
            s.attr,
            if s.large { "grande" } else { "pequeno" }
        );
    }
    println!("{shown} sprites visíveis de {}", sprites.len());
    Ok(())
}

fn flags_str(f: &Flags) -> String {
    format!(
        "M{}X{}",
        if f.m8 { 8 } else { 16 },
        if f.x8 { 8 } else { 16 }
    )
}

fn cmd_cfg(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("faltou o caminho da ROM")?;
    let rom = load_rom(path)?;
    let mut opts = cfg::Options::default();
    let (mut csv, mut ghidra) = (None, None);
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--speculative" => opts.speculative = true,
            "--tables" => {
                i += 1;
                let f = args.get(i).ok_or("--tables precisa de um arquivo")?;
                let t = fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?;
                opts.manual_tables = cfg::parse_tables(&t)?;
            }
            "--csv" => {
                i += 1;
                csv = Some(args.get(i).ok_or("--csv precisa de uma pasta")?.clone())
            }
            "--ghidra" => {
                i += 1;
                ghidra = Some(args.get(i).ok_or("--ghidra precisa de um arquivo")?.clone())
            }
            x => return Err(format!("opção desconhecida: {x}")),
        }
        i += 1;
    }

    let definite = cfg::analyze(
        &rom,
        &cfg::Options {
            speculative: false,
            ..opts.clone()
        },
    );
    let spec = cfg::analyze(
        &rom,
        &cfg::Options {
            speculative: true,
            ..opts.clone()
        },
    );
    let pct = |b: usize| 100.0 * b as f64 / rom.data.len() as f64;

    println!("== Cobertura de código ==");
    println!(
        "{:<34}{:>10}{:>14}{:>10}",
        "", "instruções", "bytes", "% da ROM"
    );
    for (name, a) in [
        ("definitiva (sem heurística)", &definite),
        ("com tabelas heurísticas", &spec),
    ] {
        println!(
            "{:<34}{:>10}{:>14}{:>9.1}%",
            name,
            a.nodes.len(),
            a.code_bytes(),
            pct(a.code_bytes())
        );
    }
    println!(
        "\nFunções (alvos de JSR/JSL + vetores): {} definitivas, {} com heurística",
        definite.functions.len(),
        spec.functions.len()
    );

    let unres = definite
        .indirect
        .iter()
        .filter(|s| s.resolution == cfg::Resolution::Unresolved)
        .count();
    let manual = definite
        .indirect
        .iter()
        .filter(|s| s.resolution == cfg::Resolution::Manual)
        .count();
    let sp_res = spec
        .indirect
        .iter()
        .filter(|s| s.resolution == cfg::Resolution::Heuristic)
        .count();
    let sp_unres = spec
        .indirect
        .iter()
        .filter(|s| s.resolution == cfg::Resolution::Unresolved)
        .count();
    println!("\n== Saltos indiretos (precisam de tabela) ==");
    println!(
        "sem heurística : {} total, {} resolvidos à mão, {} pendentes",
        definite.indirect.len(),
        manual,
        unres
    );
    println!("com heurística : {} total, {} resolvidos por heurística, {} sem tabela estática (ponteiro em RAM)", spec.indirect.len(), sp_res, sp_unres);

    println!("\n== Sinais de decodificação duvidosa (com heurística) ==");
    println!(
        "endereços ambíguos (M/X mudam o tamanho): {}",
        spec.ambiguous.len()
    );
    println!(
        "instruções sobrepostas                  : {}",
        spec.overlaps()
    );
    println!(
        "destinos fora da ROM (ex.: código em RAM): {}",
        spec.undecodable.len()
    );

    println!("\n== Código por banco (com heurística) ==");
    let mut bank_total = 0;
    for (b, bytes) in spec.per_bank() {
        bank_total += 1;
        println!(
            "banco {:02X}: {:>6} bytes de código ({:>5.1}% do banco)",
            b,
            bytes,
            100.0 * bytes as f64 / 32768.0
        );
    }
    println!(
        "{bank_total} bancos com código de {} no total",
        rom.data.len() / 0x8000
    );

    println!("\n== Saltos indiretos pendentes (declare em --tables) ==");
    for s in definite
        .indirect
        .iter()
        .filter(|s| s.resolution == cfg::Resolution::Unresolved)
        .take(25)
    {
        let t = s
            .table
            .map(|(b, a)| format!("tabela {b:02X}:{a:04X}"))
            .unwrap_or_else(|| "ponteiro em RAM".into());
        let n = spec
            .indirect
            .iter()
            .find(|x| x.at == s.at)
            .map(|x| x.targets.len())
            .unwrap_or(0);
        println!(
            "  {:02X}:{:04X}  {} {}   {}{}",
            s.at.0,
            s.at.1,
            s.mnemonic,
            s.operand,
            t,
            if n > 0 {
                format!("  (heurística achou {n} entradas)")
            } else {
                String::new()
            }
        );
    }

    let chosen = if opts.speculative { &spec } else { &definite };
    if let Some(dir) = csv {
        fs::create_dir_all(&dir).map_err(|e| format!("{dir}: {e}"))?;
        let mut f = String::from("entrada,estados_MX\n");
        for (&(b, a), st) in &chosen.functions {
            f += &format!(
                "{b:02X}:{a:04X},{}\n",
                st.iter().map(flags_str).collect::<Vec<_>>().join("|")
            );
        }
        fs::write(format!("{dir}/functions.csv"), f).map_err(|e| e.to_string())?;
        let mut g = String::from("local,instrucao,tabela,resolucao,alvos\n");
        for s in &chosen.indirect {
            let t = s
                .table
                .map(|(b, a)| format!("{b:02X}:{a:04X}"))
                .unwrap_or_default();
            let tg = s
                .targets
                .iter()
                .map(|x| format!("{x:04X}"))
                .collect::<Vec<_>>()
                .join(" ");
            g += &format!(
                "{:02X}:{:04X},{} {},{t},{:?},{tg}\n",
                s.at.0, s.at.1, s.mnemonic, s.operand, s.resolution
            );
        }
        fs::write(format!("{dir}/indirect.csv"), g).map_err(|e| e.to_string())?;
        println!(
            "\nCSVs gravados em {dir}/ (usando a análise {})",
            if opts.speculative {
                "com heurística"
            } else {
                "definitiva"
            }
        );
    }
    if let Some(file) = ghidra {
        let mut py = String::from("# Gerado por ffrs. NÃO TESTADO no seu Ghidra: rode em Window > Script Manager (Python).\n");
        py += "# Desmonta cada entrada de função com o estado M/X correto e cria a função.\n";
        py += "from java.math import BigInteger\n\nENTRADAS = [\n";
        for (&(b, a), st) in &chosen.functions {
            for f in st {
                py += &format!(
                    "    (0x{:06X}, {}, {}),\n",
                    ((0x80 | b as u32) << 16) | a as u32,
                    f.m8 as u8,
                    f.x8 as u8
                );
            }
        }
        py += "]\n\nctx = currentProgram.getProgramContext()\nmf = ctx.getRegister(\"ctx_MF\")\nxf = ctx.getRegister(\"ctx_XF\")\n";
        py += "for end, m, x in ENTRADAS:\n    a = toAddr(end)\n    if mf is not None:\n        ctx.setValue(mf, a, a, BigInteger.valueOf(m))\n";
        py += "    if xf is not None:\n        ctx.setValue(xf, a, a, BigInteger.valueOf(x))\n    disassemble(a)\n";
        py += "    if getFunctionAt(a) is None:\n        createFunction(a, None)\nprint(\"%d entradas processadas\" % len(ENTRADAS))\n";
        fs::write(&file, py).map_err(|e| format!("{file}: {e}"))?;
        println!("Script de Ghidra gravado em {file}");
    }
    Ok(())
}

fn cmd_boot(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("faltou o caminho da ROM")?;
    let mut frames = 600u32;
    let mut frames_set = false;
    let mut dump = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--dump" => {
                i += 1;
                dump = Some(args.get(i).ok_or("--dump precisa de um caminho")?.as_str());
            }
            option if option.starts_with("--") => {
                return Err(format!("opção desconhecida para boot: {option}"));
            }
            value if !frames_set => {
                frames = value
                    .parse()
                    .map_err(|_| format!("número de quadros inválido: {value}"))?;
                frames_set = true;
            }
            value => return Err(format!("argumento inesperado para boot: {value}")),
        }
        i += 1;
    }
    boot::run_command(load_rom(path)?, frames, dump)
}

fn cmd_map() {
    for r in wram::WRAM_REGIONS {
        let tag = if r.conf == wram::Confidence::Confirmado {
            "confirmado"
        } else {
            "hipótese  "
        };
        println!(
            "{:04X}-{:04X}  [{tag}]  {}",
            r.start,
            r.start + r.len - 1,
            r.name
        );
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let res = match args.first().map(String::as_str) {
        Some("info") => args
            .get(1)
            .ok_or("faltou o caminho da ROM".to_string())
            .and_then(|p| cmd_info(p)),
        Some("disasm") => cmd_disasm(&args[1..]),
        Some("wramdiff") => cmd_wramdiff(&args[1..]),
        Some("oam") => cmd_oam(&args[1..]),
        Some("cfg") => cmd_cfg(&args[1..]),
        Some("boot") => cmd_boot(&args[1..]),
        Some("map") => {
            cmd_map();
            Ok(())
        }
        _ => {
            print!("{HELP}");
            Ok(())
        }
    };
    if let Err(e) = res {
        eprintln!("erro: {e}");
        process::exit(1);
    }
}

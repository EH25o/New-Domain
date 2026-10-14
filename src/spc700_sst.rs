use crate::cpu_sst::Json;
use crate::spc700::Spc700;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct State {
    pc: u16,
    a: u8,
    x: u8,
    y: u8,
    sp: u8,
    psw: u8,
    ram: Vec<(u32, u8)>,
}

fn state(json: &mut Json<'_>) -> Result<State, String> {
    json.expect(b'{')?;
    let mut result = State::default();
    json.whitespace();
    if json.bytes.get(json.at) == Some(&b'}') {
        json.at += 1;
        return Ok(result);
    }
    loop {
        let key = json.string()?;
        json.expect(b':')?;
        match key {
            "pc" => result.pc = json.number()? as u16,
            "a" => result.a = json.number()? as u8,
            "x" => result.x = json.number()? as u8,
            "y" => result.y = json.number()? as u8,
            "sp" => result.sp = json.number()? as u8,
            "psw" => result.psw = json.number()? as u8,
            "ram" => result.ram = json.memory()?,
            _ => json.skip_value()?,
        }
        json.whitespace();
        match json.bytes.get(json.at) {
            Some(b',') => json.at += 1,
            Some(b'}') => {
                json.at += 1;
                return Ok(result);
            }
            _ => return Err(format!("expected register separator at byte {}", json.at)),
        }
    }
}

fn cycle_count(json: &mut Json<'_>) -> Result<u8, String> {
    json.expect(b'[')?;
    json.whitespace();
    if json.bytes.get(json.at) == Some(&b']') {
        json.at += 1;
        return Ok(0);
    }
    let mut count = 0u8;
    loop {
        json.skip_value()?;
        count = count.checked_add(1).ok_or("too many cycles in a vector")?;
        json.whitespace();
        match json.bytes.get(json.at) {
            Some(b',') => json.at += 1,
            Some(b']') => {
                json.at += 1;
                return Ok(count);
            }
            _ => {
                return Err(format!(
                    "expected cycle-array separator at byte {}",
                    json.at
                ))
            }
        }
    }
}

fn cases(text: &str, mut run: impl FnMut(&str, State, State, u8)) -> Result<usize, String> {
    let mut json = Json::new(text);
    json.expect(b'[')?;
    let mut count = 0;
    json.whitespace();
    if json.bytes.get(json.at) == Some(&b']') {
        return Ok(0);
    }
    loop {
        json.expect(b'{')?;
        let (mut name, mut initial, mut final_state, mut cycles) =
            (String::new(), None, None, None);
        json.whitespace();
        loop {
            let key = json.string()?;
            json.expect(b':')?;
            match key {
                "name" => name = json.string()?.to_owned(),
                "initial" => initial = Some(state(&mut json)?),
                "final" => final_state = Some(state(&mut json)?),
                "cycles" => cycles = Some(cycle_count(&mut json)?),
                _ => json.skip_value()?,
            }
            json.whitespace();
            match json.bytes.get(json.at) {
                Some(b',') => json.at += 1,
                Some(b'}') => {
                    json.at += 1;
                    break;
                }
                _ => {
                    return Err(format!(
                        "expected test-object separator at byte {}",
                        json.at
                    ))
                }
            }
        }
        run(
            &name,
            initial.ok_or_else(|| format!("{name}: missing initial state"))?,
            final_state.ok_or_else(|| format!("{name}: missing final state"))?,
            cycles.ok_or_else(|| format!("{name}: missing cycles"))?,
        );
        count += 1;
        json.whitespace();
        match json.bytes.get(json.at) {
            Some(b',') => json.at += 1,
            Some(b']') => {
                json.at += 1;
                json.whitespace();
                if json.at != json.bytes.len() {
                    return Err(format!("trailing JSON at byte {}", json.at));
                }
                return Ok(count);
            }
            _ => return Err(format!("expected test-array separator at byte {}", json.at)),
        }
    }
}

fn dataset_dir(root: &Path) -> PathBuf {
    let versioned = root.join("v1");
    if versioned.is_dir() {
        versioned
    } else {
        root.to_owned()
    }
}

#[test]
fn single_step_tests_when_available() {
    let Ok(root) = std::env::var("SST_SPC700") else {
        eprintln!("SST_SPC700 não definido: testes SingleStepTests ignorados");
        return;
    };
    let root = dataset_dir(Path::new(&root));
    let mut files: Vec<_> = fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("não foi possível ler {}: {error}", root.display()))
        .map(|entry| entry.expect("entrada do diretório").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "nenhum vetor JSON encontrado em {}",
        root.display()
    );

    let mut cpu = Spc700::new();
    for path in files {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .expect("nome JSON UTF-8");
        let opcode = u8::from_str_radix(&file_name[..2], 16)
            .unwrap_or_else(|_| panic!("nome de arquivo de opcode inválido: {file_name}"));
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("não foi possível ler {}: {error}", path.display()));
        let count = cases(&text, |name, initial, expected, expected_cycles| {
            cpu.begin_flat_test();
            for &(address, value) in &initial.ram {
                cpu.set_flat_test_byte(address as u16, value);
            }
            cpu.pc = initial.pc;
            cpu.a = initial.a;
            cpu.x = initial.x;
            cpu.y = initial.y;
            cpu.sp = initial.sp;
            cpu.psw = initial.psw;
            cpu.sleeping = false;
            cpu.stopped = false;
            let cycles = cpu.step();
            let context = format!("{name} ({file_name}, opcode ${opcode:02X})");
            assert_eq!(cpu.a, expected.a, "{context}: A");
            assert_eq!(cpu.x, expected.x, "{context}: X");
            assert_eq!(cpu.y, expected.y, "{context}: Y");
            assert_eq!(cpu.sp, expected.sp, "{context}: SP");
            assert_eq!(cpu.pc, expected.pc, "{context}: PC");
            assert_eq!(cpu.psw, expected.psw, "{context}: PSW");
            assert_eq!(cycles, expected_cycles, "{context}: cycles");
            for &(address, value) in &expected.ram {
                assert_eq!(
                    cpu.ram[address as usize], value,
                    "{context}: RAM ${address:04X}"
                );
            }
        })
        .unwrap_or_else(|error| panic!("JSON inválido em {}: {error}", path.display()));
        assert!(count > 0, "nenhum caso encontrado em {}", path.display());
        println!("{file_name}: {count} casos");
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn parses_spc700_vector_cycles_and_ram() {
        let input = r#"[{"name":"sample","initial":{"pc":4096,"a":1,"x":2,"y":3,"sp":239,"psw":2,"ram":[[4096,0]]},"final":{"pc":4097,"a":1,"x":2,"y":3,"sp":239,"psw":2,"ram":[[4096,0]]},"cycles":[[4096,0,"read"],[4097,null,"read"]]}]"#;
        let mut seen = false;
        let count = cases(input, |name, initial, final_state, cycles| {
            assert_eq!(name, "sample");
            assert_eq!(initial.pc, 4096);
            assert_eq!(initial.ram, [(4096, 0)]);
            assert_eq!(final_state.pc, 4097);
            assert_eq!(cycles, 2);
            seen = true;
        })
        .expect("valid fixture");
        assert_eq!(count, 1);
        assert!(seen);
    }
}

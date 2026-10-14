use crate::bus::Bus;
use crate::cpu::Cpu;
use crate::rom::Rom;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Registers {
    pc: u16,
    s: u16,
    p: u8,
    a: u16,
    x: u16,
    y: u16,
    dbr: u8,
    d: u16,
    pbr: u8,
    e: bool,
    ram: Vec<(u32, u8)>,
}

pub(crate) struct Json<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) at: usize,
}

impl<'a> Json<'a> {
    pub(crate) fn new(text: &'a str) -> Json<'a> {
        Json {
            bytes: text.as_bytes(),
            at: 0,
        }
    }

    pub(crate) fn whitespace(&mut self) {
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    pub(crate) fn expect(&mut self, byte: u8) -> Result<(), String> {
        self.whitespace();
        if self.bytes.get(self.at) == Some(&byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("expected {:?} at byte {}", byte as char, self.at))
        }
    }

    pub(crate) fn string(&mut self) -> Result<&'a str, String> {
        self.whitespace();
        self.expect(b'"')?;
        let start = self.at;
        while let Some(&byte) = self.bytes.get(self.at) {
            match byte {
                b'"' => {
                    let value = std::str::from_utf8(&self.bytes[start..self.at])
                        .map_err(|error| error.to_string())?;
                    self.at += 1;
                    return Ok(value);
                }
                b'\\' => return Err(format!("escaped JSON key at byte {}", self.at)),
                _ => self.at += 1,
            }
        }
        Err("unterminated JSON string".into())
    }

    pub(crate) fn number(&mut self) -> Result<u32, String> {
        self.whitespace();
        let start = self.at;
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
        if start == self.at {
            return Err(format!("expected unsigned integer at byte {}", self.at));
        }
        std::str::from_utf8(&self.bytes[start..self.at])
            .map_err(|error| error.to_string())?
            .parse()
            .map_err(|error| format!("invalid integer: {error}"))
    }

    fn skip_string(&mut self) -> Result<(), String> {
        self.whitespace();
        self.expect(b'"')?;
        while let Some(&byte) = self.bytes.get(self.at) {
            self.at += 1;
            match byte {
                b'"' => return Ok(()),
                b'\\' => {
                    let escape = *self.bytes.get(self.at).ok_or("incomplete JSON escape")?;
                    self.at += 1;
                    if escape == b'u' {
                        for _ in 0..4 {
                            self.at += 1;
                            if self.at > self.bytes.len() {
                                return Err("incomplete Unicode escape".into());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Err("unterminated JSON string".into())
    }

    pub(crate) fn skip_value(&mut self) -> Result<(), String> {
        self.whitespace();
        match self.bytes.get(self.at).copied() {
            Some(b'"') => self.skip_string(),
            Some(b'{') => {
                self.at += 1;
                self.whitespace();
                if self.bytes.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(());
                }
                loop {
                    self.skip_string()?;
                    self.expect(b':')?;
                    self.skip_value()?;
                    self.whitespace();
                    match self.bytes.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b'}') => {
                            self.at += 1;
                            return Ok(());
                        }
                        _ => return Err(format!("expected object separator at byte {}", self.at)),
                    }
                }
            }
            Some(b'[') => {
                self.at += 1;
                self.whitespace();
                if self.bytes.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(());
                }
                loop {
                    self.skip_value()?;
                    self.whitespace();
                    match self.bytes.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b']') => {
                            self.at += 1;
                            return Ok(());
                        }
                        _ => return Err(format!("expected array separator at byte {}", self.at)),
                    }
                }
            }
            Some(b'-' | b'0'..=b'9') => {
                self.at += 1;
                while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
                    self.at += 1;
                }
                Ok(())
            }
            Some(b't') => self.literal(b"true"),
            Some(b'f') => self.literal(b"false"),
            Some(b'n') => self.literal(b"null"),
            _ => Err(format!("invalid JSON value at byte {}", self.at)),
        }
    }

    fn literal(&mut self, literal: &[u8]) -> Result<(), String> {
        if self.bytes.get(self.at..self.at + literal.len()) == Some(literal) {
            self.at += literal.len();
            Ok(())
        } else {
            Err(format!("invalid JSON literal at byte {}", self.at))
        }
    }

    pub(crate) fn memory(&mut self) -> Result<Vec<(u32, u8)>, String> {
        self.expect(b'[')?;
        let mut values = Vec::new();
        self.whitespace();
        if self.bytes.get(self.at) == Some(&b']') {
            self.at += 1;
            return Ok(values);
        }
        loop {
            self.expect(b'[')?;
            let address = self.number()?;
            self.expect(b',')?;
            let value = self.number()?;
            self.expect(b']')?;
            values.push((address, value as u8));
            self.whitespace();
            match self.bytes.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(values);
                }
                _ => {
                    return Err(format!(
                        "expected memory-array separator at byte {}",
                        self.at
                    ))
                }
            }
        }
    }

    fn registers(&mut self) -> Result<Registers, String> {
        self.expect(b'{')?;
        let mut state = Registers::default();
        self.whitespace();
        if self.bytes.get(self.at) == Some(&b'}') {
            self.at += 1;
            return Ok(state);
        }
        loop {
            let key = self.string()?;
            self.expect(b':')?;
            match key {
                "pc" => state.pc = self.number()? as u16,
                "s" => state.s = self.number()? as u16,
                "p" => state.p = self.number()? as u8,
                "a" => state.a = self.number()? as u16,
                "x" => state.x = self.number()? as u16,
                "y" => state.y = self.number()? as u16,
                "dbr" => state.dbr = self.number()? as u8,
                "d" => state.d = self.number()? as u16,
                "pbr" => state.pbr = self.number()? as u8,
                "e" => state.e = self.number()? != 0,
                "ram" => state.ram = self.memory()?,
                _ => self.skip_value()?,
            }
            self.whitespace();
            match self.bytes.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(state);
                }
                _ => return Err(format!("expected register separator at byte {}", self.at)),
            }
        }
    }
}

fn cases(text: &str, mut run: impl FnMut(&str, Registers, Registers)) -> Result<usize, String> {
    let mut json = Json::new(text);
    json.expect(b'[')?;
    let mut count = 0;
    json.whitespace();
    if json.bytes.get(json.at) == Some(&b']') {
        return Ok(0);
    }
    loop {
        json.expect(b'{')?;
        let (mut name, mut initial, mut final_state) = (String::new(), None, None);
        json.whitespace();
        loop {
            let key = json.string()?;
            json.expect(b':')?;
            match key {
                "name" => name = json.string()?.to_owned(),
                "initial" => initial = Some(json.registers()?),
                "final" => final_state = Some(json.registers()?),
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
        let initial = initial.ok_or_else(|| format!("{name}: missing initial state"))?;
        let final_state = final_state.ok_or_else(|| format!("{name}: missing final state"))?;
        run(&name, initial, final_state);
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
    let Ok(root) = std::env::var("SST_65816") else {
        eprintln!("SST_65816 não definido: testes SingleStepTests ignorados");
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

    let mut bus = Bus::new(Rom::from_bytes(Vec::new()));
    bus.enable_flat_test_memory();
    for path in files {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .expect("nome JSON UTF-8");
        let opcode = u8::from_str_radix(&file_name[..2], 16)
            .unwrap_or_else(|_| panic!("nome de arquivo de opcode inválido: {file_name}"));
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("não foi possível ler {}: {error}", path.display()));
        let result = cases(&text, |name, initial, final_state| {
            let opcode_address = ((initial.pbr as u32) << 16) | initial.pc as u32;
            assert!(
                initial.ram.contains(&(opcode_address, opcode)),
                "{name} ({file_name}): opcode byte missing from initial RAM"
            );
            bus.reset_flat_test_memory();
            for &(address, value) in &initial.ram {
                bus.set_flat_test_byte(address, value);
            }
            let mut cpu = Cpu::new();
            cpu.pc = initial.pc;
            cpu.s = initial.s;
            cpu.p = initial.p;
            cpu.a = initial.a;
            cpu.x = initial.x;
            cpu.y = initial.y;
            cpu.db = initial.dbr;
            cpu.d = initial.d;
            cpu.pb = initial.pbr;
            cpu.e = initial.e;
            cpu.step(&mut bus);

            let context = format!("{} ({file_name})", name);
            assert_eq!(cpu.pc, final_state.pc, "{context}: PC");
            assert_eq!(cpu.s, final_state.s, "{context}: S");
            assert_eq!(cpu.p, final_state.p, "{context}: P");
            assert_eq!(cpu.a, final_state.a, "{context}: A");
            assert_eq!(cpu.x, final_state.x, "{context}: X");
            assert_eq!(cpu.y, final_state.y, "{context}: Y");
            assert_eq!(cpu.db, final_state.dbr, "{context}: DBR");
            assert_eq!(cpu.d, final_state.d, "{context}: D");
            assert_eq!(cpu.pb, final_state.pbr, "{context}: PBR");
            assert_eq!(cpu.e, final_state.e, "{context}: E");
            for &(address, value) in &final_state.ram {
                assert_eq!(
                    bus.flat_test_byte(address),
                    value,
                    "{context}: RAM ${address:06X}"
                );
            }
        })
        .unwrap_or_else(|error| panic!("JSON inválido em {}: {error}", path.display()));
        assert!(result > 0, "nenhum caso encontrado em {}", path.display());
        println!("{file_name}: {result} casos");
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn parses_single_step_vector_and_skips_cycles() {
        let input = r#"[{"name":"sample","initial":{"pc":4096,"s":511,"p":52,"a":4660,"x":1,"y":2,"dbr":3,"d":4,"pbr":5,"e":1,"ram":[[4096,234]]},"final":{"pc":4097,"s":511,"p":52,"a":4660,"x":1,"y":2,"dbr":3,"d":4,"pbr":5,"e":1,"ram":[[4096,234]]},"cycles":[[4096,234,"-p-remx-"]]}]"#;
        let mut seen = 0;
        let count = cases(input, |name, initial, final_state| {
            assert_eq!(name, "sample");
            assert_eq!(initial.pc, 4096);
            assert_eq!(initial.ram, [(4096, 234)]);
            assert_eq!(final_state.pc, 4097);
            seen += 1;
        })
        .expect("valid fixture");
        assert_eq!(count, 1);
        assert_eq!(seen, 1);
    }

    fn move_bytes(opcode: u8, delta: u16) {
        let mut bus = Bus::new(Rom::from_bytes(Vec::new()));
        bus.enable_flat_test_memory();
        bus.set_flat_test_byte(0x008000, opcode);
        bus.set_flat_test_byte(0x008001, 0x12);
        bus.set_flat_test_byte(0x008002, 0x34);
        bus.set_flat_test_byte(0x120020, 0xA5);
        bus.set_flat_test_byte(0x120000 | 0x20u16.wrapping_add(delta) as u32, 0x5A);

        let mut cpu = Cpu::new();
        cpu.e = false;
        cpu.p = 0;
        cpu.pb = 0;
        cpu.pc = 0x8000;
        cpu.a = 1;
        cpu.x = 0x20;
        cpu.y = 0x40;

        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x8000);
        assert_eq!(cpu.a, 0);
        assert_eq!(cpu.x, 0x20u16.wrapping_add(delta));
        assert_eq!(cpu.y, 0x40u16.wrapping_add(delta));
        assert_eq!(cpu.db, 0x34);
        assert_eq!(
            bus.flat_test_byte(0x340040),
            0xA5,
            "opcode {opcode:02X}: first byte"
        );

        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x8003);
        assert_eq!(cpu.a, 0xFFFF);
        assert_eq!(cpu.x, 0x20u16.wrapping_add(delta).wrapping_add(delta));
        assert_eq!(cpu.y, 0x40u16.wrapping_add(delta).wrapping_add(delta));
        assert_eq!(
            bus.flat_test_byte(0x340000 | 0x40u16.wrapping_add(delta) as u32),
            0x5A,
            "opcode {opcode:02X}: second byte"
        );
    }

    #[test]
    fn block_moves_repeat_one_byte_per_instruction_step() {
        move_bytes(0x54, 1);
        move_bytes(0x44, u16::MAX);
    }
}

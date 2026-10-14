//! Chip de áudio (SPC700): por enquanto só o *handshake* da ROM de boot (IPL).
//!
//! O jogo envia o programa de som pelas portas $2140-$2143 seguindo o protocolo da IPL.
//! A IPL é tratada em HLE até o driver ser enviado; então o SPC700 executa o código
//! transferido. O DSP ainda está na fase A e não produz áudio.

use crate::spc700::Spc700;
use std::collections::VecDeque;

const MASTER_HZ: u64 = 21_477_272;
const SPC_HZ: u64 = 1_024_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Boot,
    Transfer,
    Running,
}

pub struct Apu {
    pub to_cpu: [u8; 4],
    pub from_cpu: [u8; 4],
    pub state: State,
    pub ram: Vec<u8>,
    pub spc: Spc700,
    pub uploads: Vec<(u16, usize)>, // (endereço, bytes) de cada bloco
    pub entry: Option<u16>,
    pub trace: VecDeque<(u16, u8, u8)>,
    pub ipl_reentries: u64,
    addr: u16,
    expected: u8,
    cur_start: u16,
    cur_len: usize,
    pending0: Option<u8>,
    master_remainder: u64,
    cycle_debt: i64,
    last_master_cycles: u64,
}

impl Default for Apu {
    fn default() -> Self {
        Apu::new()
    }
}

impl Apu {
    pub fn new() -> Apu {
        Apu {
            to_cpu: [0xAA, 0xBB, 0, 0],
            from_cpu: [0; 4],
            state: State::Boot,
            ram: vec![0; 0x10000],
            spc: Spc700::new(),
            uploads: Vec::new(),
            entry: None,
            trace: VecDeque::with_capacity(50),
            ipl_reentries: 0,
            addr: 0,
            expected: 0,
            cur_start: 0,
            cur_len: 0,
            pending0: None,
            master_remainder: 0,
            cycle_debt: 0,
            last_master_cycles: 0,
        }
    }

    pub fn read(&mut self, port: usize) -> u8 {
        self.to_cpu[port & 3]
    }

    pub fn write(&mut self, port: usize, v: u8) {
        let p = port & 3;
        self.from_cpu[p] = v;
        if self.state == State::Running {
            self.spc.set_cpu_port(p, v);
        }
        if p == 0 {
            self.pending0 = Some(v); // processado em `sync`, depois da instrução (escritas de 16 bits)
        } else if self.state == State::Running {
            self.to_cpu[p] = v;
        }
    }

    /// Advances the sound processor to the supplied SNES master-clock position.
    pub fn sync(&mut self, master_cycles: u64) {
        let was_running = self.state == State::Running;
        if let Some(v) = self.pending0.take() {
            let arg = self.from_cpu[1];
            let target = self.from_cpu[2] as u16 | (self.from_cpu[3] as u16) << 8;
            match self.state {
                State::Boot => {
                    if v == 0xCC {
                        self.begin(arg, target, v);
                    }
                }
                State::Transfer => {
                    if v == self.expected {
                        self.ram[self.addr as usize] = arg;
                        self.addr = self.addr.wrapping_add(1);
                        self.cur_len += 1;
                        self.expected = self.expected.wrapping_add(1);
                        self.to_cpu[0] = v;
                    } else {
                        self.close_block();
                        self.begin(arg, target, v);
                    }
                }
                State::Running => {}
            }
        }

        if !was_running && self.state == State::Running {
            self.last_master_cycles = master_cycles;
            return;
        }
        if self.state != State::Running {
            return;
        }
        let elapsed_master = master_cycles.saturating_sub(self.last_master_cycles);
        self.last_master_cycles = master_cycles;
        let scaled = elapsed_master as u128 * SPC_HZ as u128 + self.master_remainder as u128;
        self.cycle_debt += (scaled / MASTER_HZ as u128) as i64;
        self.master_remainder = (scaled % MASTER_HZ as u128) as u64;

        while self.cycle_debt > 0 && !self.spc.stopped {
            let pc = self.spc.pc;
            let opcode = self.spc.ram[pc as usize];
            let cycles = self.spc.step();
            self.cycle_debt -= cycles as i64;
            if self.trace.len() == 50 {
                self.trace.pop_front();
            }
            self.trace.push_back((pc, opcode, cycles));
            if self.spc.pc == 0xFFC0 {
                self.ipl_reentries += 1;
                println!(
                    "APU: SPC700 voltou à IPL ($FFC0), reentrada #{}",
                    self.ipl_reentries
                );
                self.state = State::Boot;
                self.to_cpu = [0xAA, 0xBB, 0, 0];
                self.from_cpu = [0; 4];
                self.pending0 = None;
                self.cycle_debt = 0;
                self.master_remainder = 0;
                break;
            }
        }
        if self.state == State::Running {
            self.to_cpu = self.spc.spc_to_cpu;
        }
    }

    fn begin(&mut self, arg: u8, target: u16, echo: u8) {
        if arg != 0 {
            self.state = State::Transfer;
            self.addr = target;
            self.cur_start = target;
            self.cur_len = 0;
            self.expected = 0;
        } else {
            self.state = State::Running;
            self.entry = Some(target);
            self.spc.ram.copy_from_slice(&self.ram);
            self.spc.pc = target;
            self.spc.sleeping = false;
            self.spc.stopped = false;
            self.spc_to_cpu_init();
            self.to_cpu = [echo, 0, 0, 0];
            return;
        }
        self.to_cpu[0] = echo;
    }

    fn spc_to_cpu_init(&mut self) {
        for port in 0..4 {
            self.spc.set_cpu_port(port, self.from_cpu[port]);
        }
    }

    fn close_block(&mut self) {
        if self.cur_len > 0 {
            self.uploads.push((self.cur_start, self.cur_len));
        }
        self.cur_len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entering_ipl_resets_hle_boot_handshake() {
        let mut apu = Apu::new();
        apu.state = State::Running;
        apu.spc.pc = 0x0300;
        apu.spc.ram[0x0300..0x0303].copy_from_slice(&[0x5F, 0xC0, 0xFF]);
        apu.sync(1_000);

        assert_eq!(apu.state, State::Boot);
        assert_eq!(apu.to_cpu, [0xAA, 0xBB, 0, 0]);
        assert_eq!(apu.ipl_reentries, 1);
    }
}

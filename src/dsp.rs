const VOICES: usize = 8;
const VOICE_STRIDE: usize = 0x10;
const ENVX: usize = 0x08;
const OUTX: usize = 0x09;
const KON: usize = 0x4C;
const KOFF: usize = 0x5C;
const ENDX: usize = 0x7C;

#[derive(Clone, Copy, Default)]
struct Voice {
    active: bool,
    key_off: bool,
    envelope: u8,
    envelope_clock: u16,
}

#[derive(Clone)]
pub struct Dsp {
    regs: [u8; 128],
    voices: [Voice; VOICES],
    sample_remainder: u8,
    pub samples_clocked: u64,
}

impl Default for Dsp {
    fn default() -> Self {
        Self::new()
    }
}

impl Dsp {
    pub fn new() -> Self {
        Self {
            regs: [0; 128],
            voices: [Voice::default(); VOICES],
            sample_remainder: 0,
            samples_clocked: 0,
        }
    }

    pub fn read(&self, address: u8) -> u8 {
        self.regs[(address & 0x7F) as usize]
    }

    pub fn write(&mut self, address: u8, value: u8) {
        let address = (address & 0x7F) as usize;
        match address {
            KON => {
                self.regs[KON] = value;
                for voice in 0..VOICES {
                    if value & (1 << voice) != 0 {
                        self.voices[voice] = Voice {
                            active: true,
                            key_off: false,
                            envelope: 0,
                            envelope_clock: 0,
                        };
                        self.regs[ENDX] &= !(1 << voice);
                        self.regs[voice * VOICE_STRIDE + ENVX] = 0;
                        self.regs[voice * VOICE_STRIDE + OUTX] = 0;
                    }
                }
            }
            KOFF => {
                self.regs[KOFF] = value;
                for voice in 0..VOICES {
                    if value & (1 << voice) != 0 {
                        self.voices[voice].key_off = true;
                    }
                }
            }
            0x6C => {
                self.regs[address] = value;
                if value & 0x80 != 0 {
                    self.voices = [Voice::default(); VOICES];
                    self.regs[ENDX] = 0;
                    for voice in 0..VOICES {
                        self.regs[voice * VOICE_STRIDE + ENVX] = 0;
                        self.regs[voice * VOICE_STRIDE + OUTX] = 0;
                    }
                }
            }
            ENDX => {
                self.regs[ENDX] = 0;
            }
            _ => self.regs[address] = value,
        }
    }

    /// Advances the DSP's 32-cycle sample clock. Phase A tracks voice state but emits no audio.
    pub fn clock(&mut self, cycles: u32) {
        let elapsed = self.sample_remainder as u32 + cycles;
        let samples = elapsed / 32;
        self.sample_remainder = (elapsed % 32) as u8;
        self.samples_clocked += samples as u64;

        for _ in 0..samples {
            for voice in 0..VOICES {
                self.clock_voice(voice);
            }
        }
    }

    pub fn voice_active(&self, voice: usize) -> bool {
        self.voices[voice].active
    }

    fn clock_voice(&mut self, voice: usize) {
        let state = &mut self.voices[voice];
        if !state.active {
            return;
        }

        let base = voice * VOICE_STRIDE;
        if state.key_off {
            state.envelope = state.envelope.saturating_sub(1);
            if state.envelope == 0 {
                state.active = false;
            }
        } else {
            let adsr_enabled = self.regs[base + 0x05] & 0x80 != 0;
            let rate = if adsr_enabled {
                (self.regs[base + 0x05] & 0x0F) as u16 + 1
            } else {
                ((self.regs[base + 0x07] >> 5) & 0x07) as u16 + 1
            };
            state.envelope_clock += 1;
            if state.envelope_clock >= rate {
                state.envelope_clock = 0;
                state.envelope = state.envelope.saturating_add(1).min(0x7F);
            }
        }
        self.regs[base + ENVX] = state.envelope;
        self.regs[base + OUTX] = state.envelope;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_clock_advances_once_per_32_cycles() {
        let mut dsp = Dsp::new();
        dsp.clock(31);
        assert_eq!(dsp.samples_clocked, 0);
        dsp.clock(1);
        assert_eq!(dsp.samples_clocked, 1);
        dsp.clock(96);
        assert_eq!(dsp.samples_clocked, 4);
    }

    #[test]
    fn key_on_and_key_off_update_voice_envelope_registers() {
        let mut dsp = Dsp::new();
        dsp.write(KON as u8, 0b0000_0101);
        assert!(dsp.voice_active(0));
        assert!(dsp.voice_active(2));

        dsp.clock(32 * 8);
        assert_eq!(dsp.read(ENVX as u8), 8);
        assert_eq!(dsp.read((2 * VOICE_STRIDE + ENVX) as u8), 8);

        dsp.write(KOFF as u8, 0b0000_0001);
        assert!(dsp.voice_active(0));
        dsp.clock(32);
        assert_eq!(dsp.read(ENVX as u8), 7);
        dsp.clock(32 * 7);
        assert!(!dsp.voice_active(0));
        assert_eq!(dsp.read(ENVX as u8), 0);
        assert!(dsp.voice_active(2));
    }

    #[test]
    fn flg_soft_reset_clears_voices_and_endx() {
        let mut dsp = Dsp::new();
        dsp.write(KON as u8, 1);
        dsp.regs[ENDX] = 1;
        dsp.write(0x6C, 0x80);
        assert!(!dsp.voice_active(0));
        assert_eq!(dsp.read(ENDX as u8), 0);
        assert_eq!(dsp.read(ENVX as u8), 0);
    }

    #[test]
    fn endx_is_cleared_by_writing_it() {
        let mut dsp = Dsp::new();
        dsp.regs[ENDX] = 0xA5;
        dsp.write(ENDX as u8, 0xFF);
        assert_eq!(dsp.read(ENDX as u8), 0);
    }
}

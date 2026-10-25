//! Every sender's voice, decoded and mixed into the frame the output device plays next.
//!
//! The render callback owns the mixer: it inserts the frames the network task hands over and
//! renders one 10 ms frame at a time. All voices and their decoders exist from the start, so
//! nothing here allocates after construction.

use bukospeak_proto::MemberId;
use bukospeak_proto::audio::{FRAME_SAMPLES, SAMPLE_RATE};
use opus::{Channels, Decoder};

use crate::jitter::{self, JitterBuffer, Packet, Playout};

/// The voices a mixer holds: the server admits 16 members, and one of them is this one.
pub const VOICES: usize = 15;

/// Where the limiter starts to bend the mix; below it the mix passes unchanged.
const LIMITER_KNEE: f32 = 0.8;

pub struct Mixer {
    voices: Box<[Voice]>,
    decoded: [f32; FRAME_SAMPLES],
}

struct Voice {
    member: Option<MemberId>,
    buffer: JitterBuffer,
    decoder: Decoder,
    undecodable: u64,
}

/// How a member's voice fares, for diagnostics.
#[cfg_attr(not(test), expect(dead_code, reason = "the apps show it once they have a diagnostics view"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceStats {
    pub jitter: jitter::Stats,
    /// The frames its jitter buffer aims to hold.
    pub target: u32,
    /// Frames the decoder rejected, or that weren't 10 ms long, and were left silent.
    pub undecodable: u64,
}

impl Mixer {
    pub fn new() -> Result<Self, opus::Error> {
        let voices = (0..VOICES)
            .map(|_| {
                Ok(Voice {
                    member: None,
                    buffer: JitterBuffer::default(),
                    decoder: Decoder::new(SAMPLE_RATE, Channels::Mono)?,
                    undecodable: 0,
                })
            })
            .collect::<Result<_, opus::Error>>()?;
        Ok(Self { voices, decoded: [0.0; FRAME_SAMPLES] })
    }

    /// Hands a frame to its sender's voice, giving the sender a free voice on its first frame.
    /// False if every voice is taken by someone else.
    pub fn insert(&mut self, from: MemberId, packet: Packet) -> bool {
        let voice = match self.voices.iter().position(|voice| voice.member == Some(from)) {
            Some(index) => &mut self.voices[index],
            None => match self.voices.iter_mut().find(|voice| voice.member.is_none()) {
                Some(voice) => {
                    voice.member = Some(from);
                    voice
                }
                None => return false,
            },
        };
        voice.buffer.insert(packet);
        true
    }

    /// Frees a member's voice once it left the channel.
    pub fn remove(&mut self, member: MemberId) {
        if let Some(voice) = self.voices.iter_mut().find(|voice| voice.member == Some(member)) {
            voice.reset();
            voice.member = None;
        }
    }

    /// Forgets every voice's frames and decoder state, as after the audio engine was interrupted.
    pub fn flush(&mut self) {
        self.voices.iter_mut().for_each(Voice::reset);
    }

    /// Mixes the next 10 ms of every voice into `mix`.
    pub fn render(&mut self, mix: &mut [f32; FRAME_SAMPLES]) {
        mix.fill(0.0);
        for voice in self.voices.iter_mut().filter(|voice| voice.member.is_some()) {
            let decoded = match voice.buffer.pop() {
                Playout::Silent => continue,
                Playout::Frame(opus) => voice.decoder.decode_float(opus, &mut self.decoded, false),
                Playout::Fec(opus) => voice.decoder.decode_float(opus, &mut self.decoded, true),
                // An empty packet asks Opus to conceal the frame.
                Playout::Conceal => voice.decoder.decode_float(&[], &mut self.decoded, false),
            };
            if decoded.is_ok_and(|samples| samples == FRAME_SAMPLES) {
                mix.iter_mut().zip(&self.decoded).for_each(|(mixed, sample)| *mixed += sample);
            } else {
                voice.undecodable += 1;
            }
        }
        mix.iter_mut().for_each(|sample| *sample = limit(*sample));
    }

    /// The members whose voice is playing.
    pub fn speaking(&self) -> impl Iterator<Item = MemberId> + '_ {
        self.voices.iter().filter(|voice| voice.buffer.is_playing()).filter_map(|voice| voice.member)
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "the apps show it once they have a diagnostics view"))]
    pub fn stats(&self, member: MemberId) -> Option<VoiceStats> {
        let voice = self.voices.iter().find(|voice| voice.member == Some(member))?;
        Some(VoiceStats { jitter: voice.buffer.stats(), target: voice.buffer.target(), undecodable: voice.undecodable })
    }
}

impl Voice {
    fn reset(&mut self) {
        self.buffer.flush();
        // libopus resets a decoder it created without fail.
        self.decoder.reset_state().expect("resetting a decoder succeeds");
    }
}

/// Bends samples beyond the knee smoothly towards ±1, so a loud mix never clips: linear below
/// the knee, and tanh above it, which meets it with the same slope.
fn limit(sample: f32) -> f32 {
    let magnitude = sample.abs();
    if magnitude <= LIMITER_KNEE {
        return sample;
    }
    let headroom = 1.0 - LIMITER_KNEE;
    (LIMITER_KNEE + headroom * ((magnitude - LIMITER_KNEE) / headroom).tanh()).copysign(sample)
}

#[cfg(test)]
mod tests {
    use bukospeak_proto::audio::{FRAME_DURATION, Level};
    use opus::{Application, Bitrate, Encoder};

    use super::*;

    const FRAME_US: u64 = FRAME_DURATION.as_micros() as u64;

    /// A talkspurt of a sine as the sender would encode it, its frames arriving 10 ms apart.
    fn talkspurt(frames: u32, frequency: f32, amplitude: f32) -> Vec<Packet> {
        let mut encoder = Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Voip).unwrap();
        encoder.set_bitrate(Bitrate::Bits(48_000)).unwrap();
        encoder.set_inband_fec(true).unwrap();
        encoder.set_packet_loss_perc(10).unwrap();
        (0..frames)
            .map(|index| {
                let samples: Vec<f32> = (0..FRAME_SAMPLES)
                    .map(|n| {
                        let t = (index as usize * FRAME_SAMPLES + n) as f32 / SAMPLE_RATE as f32;
                        amplitude * (std::f32::consts::TAU * frequency * t).sin()
                    })
                    .collect();
                let opus = encoder.encode_vec_float(&samples, 320).unwrap();
                Packet::new(index, index + 1 == frames, Level::of(&samples), u64::from(index) * FRAME_US, &opus).unwrap()
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// Plays the senders' frames as they arrive, one 10 ms frame at a time, and returns each
    /// rendered frame's RMS and who spoke during it.
    fn play(mixer: &mut Mixer, senders: &[(MemberId, Vec<Packet>)], frames: u64) -> Vec<(f32, Vec<MemberId>)> {
        let mut mix = [0.0; FRAME_SAMPLES];
        (0..frames)
            .map(|tick| {
                for (sender, packets) in senders {
                    for packet in packets.iter().filter(|packet| packet.arrival_us == tick * FRAME_US) {
                        assert!(mixer.insert(*sender, *packet));
                    }
                }
                mixer.render(&mut mix);
                assert!(mix.iter().all(|sample| sample.abs() <= 1.0), "the mix clipped");
                (rms(&mix), mixer.speaking().collect())
            })
            .collect()
    }

    #[test]
    fn a_voice_plays_while_its_talkspurt_lasts() {
        let mut mixer = Mixer::new().unwrap();
        let speaker = MemberId(7);
        let rendered = play(&mut mixer, &[(speaker, talkspurt(50, 440.0, 0.5))], 60);
        // A sine of amplitude 0.5 has an RMS of 0.35. Playout starts a frame late, at its target
        // of two frames, and Opus fades in over the first.
        for (rms, speaking) in &rendered[3..50] {
            assert!((0.33..0.38).contains(rms), "rms {rms}");
            assert_eq!(speaking, &[speaker]);
        }
        for (rms, speaking) in &rendered[52..] {
            assert_eq!((*rms, speaking.len()), (0.0, 0));
        }
        assert_eq!(mixer.stats(speaker).unwrap().jitter, jitter::Stats { played: 50, ..jitter::Stats::default() });
    }

    #[test]
    fn voices_are_mixed_and_limited() {
        let mut mixer = Mixer::new().unwrap();
        let senders = [(MemberId(1), talkspurt(50, 440.0, 0.9)), (MemberId(2), talkspurt(50, 660.0, 0.9))];
        let (rms, speaking) = &play(&mut mixer, &senders, 50)[20];
        // Each sine alone has an RMS of 0.64; their sum peaks at 1.8, which the limiter bends
        // below full scale (`play` checks every sample), so the mix is louder but not twice so.
        assert!((0.64..0.9).contains(rms), "rms {rms}");
        assert_eq!(speaking, &[MemberId(1), MemberId(2)]);
    }

    #[test]
    fn a_lost_frame_is_filled_in() {
        let mut mixer = Mixer::new().unwrap();
        let packets = talkspurt(50, 440.0, 0.5).into_iter().filter(|packet| packet.index != 20).collect();
        let rendered = play(&mut mixer, &[(MemberId(1), packets)], 50);
        // Frame 20 plays at tick 21, recovered from frame 21's FEC.
        assert!(rendered[21].0 > 0.2, "the lost frame is near silent: {}", rendered[21].0);
        let stats = mixer.stats(MemberId(1)).unwrap();
        assert_eq!((stats.jitter.recovered, stats.undecodable), (1, 0));
    }

    #[test]
    fn senders_beyond_the_voices_are_turned_away_until_one_is_free() {
        let mut mixer = Mixer::new().unwrap();
        let packet = talkspurt(1, 440.0, 0.5)[0];
        for id in 1..=VOICES as u16 {
            assert!(mixer.insert(MemberId(id), packet));
        }
        let newcomer = MemberId(VOICES as u16 + 1);
        assert!(!mixer.insert(newcomer, packet));
        mixer.remove(MemberId(1));
        assert!(mixer.insert(newcomer, packet));
    }

    #[test]
    fn flush_silences_every_voice() {
        let mut mixer = Mixer::new().unwrap();
        let packets = talkspurt(50, 440.0, 0.5);
        let early: Vec<Packet> = packets.iter().copied().filter(|packet| packet.index < 10).collect();
        play(&mut mixer, &[(MemberId(1), early)], 10);
        mixer.flush();
        let rendered = play(&mut mixer, &[], 5);
        assert!(rendered.iter().all(|(rms, speaking)| *rms == 0.0 && speaking.is_empty()));
    }

    #[test]
    fn an_undecodable_frame_is_counted_and_left_silent() {
        let mut mixer = Mixer::new().unwrap();
        // A TOC byte announcing a 20 ms frame, which the mixer can't fit into 10 ms.
        let garbage = Packet::new(0, true, Level::new(20), 0, &[0xff, 0xff, 0xff]).unwrap();
        mixer.insert(MemberId(1), garbage);
        assert_eq!(play(&mut mixer, &[], 1)[0].0, 0.0);
        assert_eq!(mixer.stats(MemberId(1)).unwrap().undecodable, 1);
    }

    #[test]
    fn the_limiter_passes_quiet_samples_and_bends_loud_ones_below_full_scale() {
        assert_eq!(limit(0.5), 0.5);
        assert_eq!(limit(-LIMITER_KNEE), -LIMITER_KNEE);
        for loud in [0.9, 1.0, 2.0, 100.0] {
            // tanh reaches 1.0 in f32 for large arguments, so the bound is inclusive.
            assert!(limit(loud) > LIMITER_KNEE && limit(loud) <= 1.0, "limit({loud}) = {}", limit(loud));
            assert_eq!(limit(-loud), -limit(loud));
        }
        assert!(limit(0.9) < limit(1.0) && limit(1.0) < limit(2.0));
    }
}

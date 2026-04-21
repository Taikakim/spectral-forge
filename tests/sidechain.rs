use spectral_forge::dsp::pipeline::resolve_sc_source;
use spectral_forge::params::{FxChannelTarget, ScChannel, StereoLink};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Expected { L, R, LR, M, S }

fn exp(c: ScChannel, link: StereoLink, target: FxChannelTarget, ch: usize) -> Expected {
    match resolve_sc_source(c, link, target, ch) {
        spectral_forge::dsp::pipeline::ScSource::L => Expected::L,
        spectral_forge::dsp::pipeline::ScSource::R => Expected::R,
        spectral_forge::dsp::pipeline::ScSource::LR => Expected::LR,
        spectral_forge::dsp::pipeline::ScSource::M => Expected::M,
        spectral_forge::dsp::pipeline::ScSource::S => Expected::S,
    }
}

#[test]
fn follow_linked_is_lr() {
    assert_eq!(exp(ScChannel::Follow, StereoLink::Linked, FxChannelTarget::All, 0), Expected::LR);
    assert_eq!(exp(ScChannel::Follow, StereoLink::Linked, FxChannelTarget::All, 1), Expected::LR);
}

#[test]
fn follow_independent_pairs_channels() {
    assert_eq!(exp(ScChannel::Follow, StereoLink::Independent, FxChannelTarget::All, 0), Expected::L);
    assert_eq!(exp(ScChannel::Follow, StereoLink::Independent, FxChannelTarget::All, 1), Expected::R);
}

#[test]
fn follow_midside_respects_target() {
    assert_eq!(exp(ScChannel::Follow, StereoLink::MidSide, FxChannelTarget::Mid,  0), Expected::M);
    assert_eq!(exp(ScChannel::Follow, StereoLink::MidSide, FxChannelTarget::Side, 1), Expected::S);
    assert_eq!(exp(ScChannel::Follow, StereoLink::MidSide, FxChannelTarget::All,  0), Expected::LR);
}

#[test]
fn explicit_channels_always_apply_literally() {
    for link in [StereoLink::Linked, StereoLink::Independent, StereoLink::MidSide] {
        for target in [FxChannelTarget::All, FxChannelTarget::Mid, FxChannelTarget::Side] {
            for ch in [0usize, 1] {
                assert_eq!(exp(ScChannel::L, link, target, ch), Expected::L);
                assert_eq!(exp(ScChannel::R, link, target, ch), Expected::R);
                assert_eq!(exp(ScChannel::LR, link, target, ch), Expected::LR);
                assert_eq!(exp(ScChannel::M, link, target, ch), Expected::M);
                assert_eq!(exp(ScChannel::S, link, target, ch), Expected::S);
            }
        }
    }
}

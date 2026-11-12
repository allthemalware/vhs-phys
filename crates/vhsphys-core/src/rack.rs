//! A rack of decks: generations of either raster joined by explicit links.
//!
//! `pipeline::Chain` dubs through decks of one engine and hands each generation's decoded
//! baseband straight to the next. A `Rack` does the same between decks that need not share a
//! raster. Three things a `Chain` leaves implicit are stated here:
//!
//! * every hand-over is a `SignalChunk`: samples tagged with their raster, their form and
//!   their programme time. A deck refuses a chunk of another raster; nothing is resized or
//!   relabelled to make it fit;
//! * every pair of neighbouring generations is joined by a `LinkSpec`, checked before any
//!   picture is processed. A change of raster needs a `standards_converter` link with a named
//!   profile; without one the rack is refused, with the link and the two ports in the message;
//! * each generation runs on its own clock. Frame counts are not shared: a generation starts
//!   at the first frame of its raster at or after the start of the generation before it, and
//!   the converter derives the output frames from the programme interval.
//!
//! Only the last generation's signal reaches a picture tube. A link carries signal samples
//! in and out and never a `Frame`.
//!
//! With one raster throughout and every link `direct_yc_reference`, a rack gives the frames
//! of a `Chain` with the same parameters.
//!
//! See `docs/design/HETEROGENEOUS_BOUNCE.md`.

use crate::automation::AutomationSchedule;
use crate::convert::{ConverterProfile, ConverterRecord, StandardsConverter};
use crate::frame::Frame;
use crate::signal::{CompositeSystem, LinkTiming, Planes, Raster, Rational, SignalChunk, SignalForm, SignalPort};
use crate::standard::TvStandard;
use crate::{ntsc, pal};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Frames an earlier generation plays back at a time into its link. Batches keep the engine's
/// parallel stages busy; the size has no effect on the result.
const FORWARD_BATCH: usize = 8;

/// Config keys of the rack itself; everything else at the top level is common to every generation.
const GENERATION_KEY: &str = "generation";
const LINK_KEY: &str = "link";
/// Keys of a `[[generation]]` table that are not parameters of its deck.
const NAME_KEY: &str = "name";
const PRESET_KEY: &str = "preset";
const SEED_KEY: &str = "seed";
const STANDARD_KEY: &str = "standard";

/// Multiplier that spreads the generation index over the seed, as `config::generation_params` does.
const SEED_SPREAD: u64 = 0x9E37_79B9_7F4A_7C15;

// ------------------------------------------------------------------ parameters

/// The parameters of one generation, for the engine that plays its raster.
#[derive(Clone, Debug)]
pub enum StageParams {
    L525(Box<ntsc::pipeline::Params>),
    L625(Box<pal::pipeline::Params>),
}

impl StageParams {
    pub fn raster(&self) -> Raster {
        match self {
            StageParams::L525(_) => Raster::L525,
            StageParams::L625(_) => Raster::L625,
        }
    }

    /// Recorded mode and playback output in one name: `ntsc`, `pal`, or `pal60` (an NTSC
    /// tape whose deck plays PAL colour on the 525-line raster).
    pub fn standard(&self) -> TvStandard {
        match self {
            StageParams::L525(p) => p.standard,
            StageParams::L625(p) => p.standard,
        }
    }

    pub fn seed(&self) -> u64 {
        match self {
            StageParams::L525(p) => p.seed,
            StageParams::L625(p) => p.seed,
        }
    }

    /// The composite system this deck's video output carries.
    pub fn output_system(&self) -> CompositeSystem {
        match self.standard() {
            TvStandard::Ntsc => CompositeSystem::MNtsc,
            TvStandard::Pal => CompositeSystem::BgPal,
            TvStandard::Pal60 => CompositeSystem::Pal60,
        }
    }

    /// The composite system this deck's record input takes: that of its tape format. A deck
    /// that plays PAL-60 still records M/NTSC.
    pub fn input_system(&self) -> CompositeSystem {
        match self.raster() {
            Raster::L525 => CompositeSystem::MNtsc,
            Raster::L625 => CompositeSystem::BgPal,
        }
    }

    fn set_composite_off(&mut self) {
        match self {
            StageParams::L525(p) => p.composite.enabled = false,
            StageParams::L625(p) => p.composite.enabled = false,
        }
    }
}

/// One generation of a rack.
#[derive(Clone, Debug)]
pub struct StageSpec {
    pub name: String,
    pub params: StageParams,
}

/// The Y/C separator at the receiving end of a composite link.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Separator {
    #[default]
    Notch,
    Comb1h,
    Comb2h,
}

/// What joins a generation's playback output to the next generation's record input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LinkSpec {
    /// The numerical reference: decoded luma and colour difference handed over unchanged. No
    /// connector, no cable, no composite effects, no change of raster. It is what `Chain` does.
    DirectYcReference,
    /// Separate luma and chroma conductors. The two never share a wire, so there is no
    /// cross-colour or cross-luminance; no cable or connector loss is modelled, because none
    /// has a stated profile yet. The chroma is still on a subcarrier, so the colour systems of
    /// the two decks must agree.
    SeparateYc,
    /// One composite waveform: the upstream deck's output is encoded on its subcarrier and the
    /// downstream deck's input separates it again, with the cross-effects of `separator`. No
    /// cable or load loss is modelled.
    Composite {
        #[serde(default)]
        separator: Separator,
    },
    /// A change of raster by a named algorithm. The profile has no default.
    StandardsConverter { profile: ConverterProfile },
}

impl LinkSpec {
    pub fn type_name(&self) -> &'static str {
        match self {
            LinkSpec::DirectYcReference => "direct_yc_reference",
            LinkSpec::SeparateYc => "separate_yc",
            LinkSpec::Composite { .. } => "composite",
            LinkSpec::StandardsConverter { .. } => "standards_converter",
        }
    }
}

/// A link with its name.
#[derive(Clone, Debug)]
pub struct NamedLink {
    pub name: String,
    pub spec: LinkSpec,
}

/// Generations and the links between them: one link fewer than generations.
#[derive(Clone, Debug)]
pub struct RackPlan {
    pub stages: Vec<StageSpec>,
    pub links: Vec<NamedLink>,
}

impl RackPlan {
    /// A plan with the direct reference link between every pair of generations.
    pub fn direct(stages: Vec<StageParams>) -> Self {
        let links = (1..stages.len()).map(|i| NamedLink { name: default_link_name(i - 1), spec: LinkSpec::DirectYcReference }).collect();
        let stages = stages.into_iter().enumerate().map(|(i, params)| StageSpec { name: default_stage_name(i), params }).collect();
        Self { stages, links }
    }

    /// The two ports of every link, or why the rack cannot be built. Nothing is converted to
    /// make an edge fit: an edge that does not fit is an error that names it.
    pub fn validate(&self) -> Result<Vec<(SignalPort, SignalPort)>, String> {
        if self.stages.is_empty() {
            return Err("a rack needs at least one generation".into());
        }
        if self.links.len() + 1 != self.stages.len() {
            return Err(format!("{} generations need {} links, the rack has {}", self.stages.len(), self.stages.len() - 1, self.links.len()));
        }
        let mut ports = Vec::new();
        for (i, link) in self.links.iter().enumerate() {
            let (up, down) = (&self.stages[i], &self.stages[i + 1]);
            let (ru, rd) = (up.params.raster(), down.params.raster());
            let edge = format!("link {i} '{}' ({} '{}' -> '{}')", link.name, link.spec.type_name(), up.name, down.name);
            let same_raster = || {
                if ru == rd {
                    Ok(())
                } else {
                    Err(format!(
                        "{edge}: the output is {} and the record input is {}; a {} link does not change the raster, use a standards_converter link with a named profile",
                        ru.name(),
                        rd.name(),
                        link.spec.type_name()
                    ))
                }
            };
            let same_colour = || {
                let (out, inp) = (up.params.output_system(), down.params.input_system());
                if out == inp {
                    Ok(())
                } else {
                    Err(format!("{edge}: the output is {} and the record input takes {}; the colour systems differ", out.name(), inp.name()))
                }
            };
            let pair = match link.spec {
                LinkSpec::DirectYcReference => {
                    same_raster()?;
                    (SignalPort::reference(ru), SignalPort::reference(rd))
                }
                LinkSpec::SeparateYc => {
                    same_raster()?;
                    same_colour()?;
                    (SignalPort { raster: ru, form: SignalForm::SeparateYc }, SignalPort { raster: rd, form: SignalForm::SeparateYc })
                }
                LinkSpec::Composite { .. } => {
                    same_raster()?;
                    same_colour()?;
                    let form = SignalForm::Composite(up.params.output_system());
                    (SignalPort { raster: ru, form }, SignalPort { raster: rd, form })
                }
                LinkSpec::StandardsConverter { profile } => {
                    if ru == rd {
                        return Err(format!("{edge}: both generations are {}; a standards converter ({}) joins two different rasters", ru.name(), profile.name()));
                    }
                    (SignalPort::reference(ru), SignalPort::reference(rd))
                }
            };
            ports.push(pair);
        }
        Ok(ports)
    }

    /// Read a rack from config text, or `None` when the text describes no rack (it has no
    /// `[[generation]]` table) and the run is an ordinary one.
    ///
    /// The top level of the text, without its `[[generation]]` and `[[link]]` tables, is common
    /// to every generation. Each `[[generation]]` table is merged over it: `standard` picks the
    /// engine, `preset` (default: `preset`) and `[generation.condition]` the condition, the rest
    /// are parameter overrides; `name` and `seed` are the generation's own. `overrides` is more
    /// TOML applied to every generation last (the `--set` items of a run). Seeds follow
    /// `config::generation_params`: the first generation takes the chain's seed, each later one
    /// derives its own from it.
    ///
    /// `composite.enabled` describes the connection to the television at the end of the rack,
    /// so it is cleared on every generation but the last; a composite hop between decks is a
    /// `composite` link.
    pub fn from_config(preset: &str, text: &str, seed: u64, overrides: &str) -> Result<Option<RackPlan>, String> {
        let mut base: toml::Table = toml::from_str(text).map_err(|e| format!("config: {e}"))?;
        let generations = match base.remove(GENERATION_KEY) {
            Some(toml::Value::Array(a)) => a,
            Some(_) => return Err(format!("config: {GENERATION_KEY} must be an array of tables ([[{GENERATION_KEY}]])")),
            None => {
                if base.contains_key(LINK_KEY) {
                    return Err(format!("config: [[{LINK_KEY}]] tables need [[{GENERATION_KEY}]] tables to join"));
                }
                return Ok(None);
            }
        };
        let links = match base.remove(LINK_KEY) {
            Some(toml::Value::Array(a)) => Some(a),
            Some(_) => return Err(format!("config: {LINK_KEY} must be an array of tables ([[{LINK_KEY}]])")),
            None => None,
        };
        if generations.is_empty() {
            return Err("a rack needs at least one generation".into());
        }
        let chain_seed = match base.get(SEED_KEY) {
            Some(toml::Value::Integer(s)) => *s as u64,
            _ => seed,
        };

        let last = generations.len() - 1;
        let mut stages = Vec::with_capacity(generations.len());
        for (g, entry) in generations.into_iter().enumerate() {
            let toml::Value::Table(mut entry) = entry else {
                return Err(format!("config: {GENERATION_KEY} {g} must be a table"));
            };
            let name = match entry.remove(NAME_KEY) {
                Some(toml::Value::String(s)) => s,
                Some(_) => return Err(format!("config: the name of {GENERATION_KEY} {g} must be a string")),
                None => default_stage_name(g),
            };
            let stage_preset = match entry.remove(PRESET_KEY) {
                Some(toml::Value::String(s)) => s,
                Some(_) => return Err(format!("config: the preset of '{name}' must be a string")),
                None => preset.to_string(),
            };
            let own_seed = match entry.remove(SEED_KEY) {
                Some(toml::Value::Integer(s)) => Some(s as u64),
                Some(_) => return Err(format!("config: the seed of '{name}' must be a whole number")),
                None => None,
            };
            let mut table = base.clone();
            // The first generation keeps a top-level seed in its text, as a plain run does;
            // a later one must not, or it would replace the seed derived for it.
            if g > 0 || own_seed.is_some() {
                table.remove(SEED_KEY);
            }
            merge_tables(&mut table, entry);
            let standard = match table.get(STANDARD_KEY) {
                Some(toml::Value::String(s)) => {
                    TvStandard::ALL.into_iter().find(|t| t.name() == s).ok_or_else(|| format!("'{name}': unknown television standard '{s}' (use ntsc, pal or pal60)"))?
                }
                Some(_) => return Err(format!("'{name}': standard must be a name (ntsc, pal or pal60)")),
                None => TvStandard::Ntsc,
            };
            let stage_seed = match own_seed {
                Some(s) => s,
                None if g == 0 => seed,
                None => crate::rng::mix(chain_seed ^ (g as u64).wrapping_mul(SEED_SPREAD)),
            };
            let stage_text = toml::to_string(&table).map_err(|e| format!("config: {e}"))?;
            let fixed_seed = (g > 0 || own_seed.is_some()).then_some(stage_seed);
            let mut params = build_stage(standard, &stage_preset, &stage_text, stage_seed, overrides, fixed_seed).map_err(|e| format!("'{name}': {e}"))?;
            if g != last {
                params.set_composite_off();
            }
            stages.push(StageSpec { name, params });
        }

        let links = match links {
            None => (0..last).map(|i| NamedLink { name: default_link_name(i), spec: LinkSpec::DirectYcReference }).collect(),
            Some(entries) => {
                let mut out = Vec::with_capacity(entries.len());
                for (i, entry) in entries.into_iter().enumerate() {
                    let toml::Value::Table(mut entry) = entry else {
                        return Err(format!("config: {LINK_KEY} {i} must be a table"));
                    };
                    let name = match entry.remove(NAME_KEY) {
                        Some(toml::Value::String(s)) => s,
                        Some(_) => return Err(format!("config: the name of {LINK_KEY} {i} must be a string")),
                        None => default_link_name(i),
                    };
                    let spec: LinkSpec = toml::Value::Table(entry).try_into().map_err(|e| {
                        format!(
                            "link {i} '{name}': {e} (type is direct_yc_reference, separate_yc, composite or standards_converter; a standards_converter names its profile: {})",
                            ConverterProfile::ALL.map(ConverterProfile::name).join(", ")
                        )
                    })?;
                    out.push(NamedLink { name, spec });
                }
                out
            }
        };
        let plan = RackPlan { stages, links };
        plan.validate()?;
        Ok(Some(plan))
    }
}

fn default_stage_name(i: usize) -> String {
    format!("generation {i}")
}

fn default_link_name(i: usize) -> String {
    format!("{i} to {}", i + 1)
}

/// `over` on top of `base`, table by table.
fn merge_tables(base: &mut toml::Table, over: toml::Table) {
    for (k, v) in over {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge_tables(b, o),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

/// Build one generation on the engine that plays `standard`. `fixed_seed` pins the seed after
/// the overrides, for a generation whose seed is its own and not the text's.
fn build_stage(standard: TvStandard, preset: &str, text: &str, seed: u64, overrides: &str, fixed_seed: Option<u64>) -> Result<StageParams, String> {
    macro_rules! build {
        ($engine:ident, $variant:ident) => {{
            let p = $engine::config::build(Some(preset), text, seed)?;
            let mut p = $engine::config::apply_overrides(&p, overrides)?;
            if let Some(s) = fixed_seed {
                p.seed = s;
                p.interference.seed = s;
            }
            StageParams::$variant(Box::new(p))
        }};
    }
    Ok(if standard.is_625() { build!(pal, L625) } else { build!(ntsc, L525) })
}

// ------------------------------------------------------------------ decks

/// A deck of either engine behind one interface: tagged chunks in, tagged chunks or frames out.
enum Deck {
    L525(Box<ntsc::pipeline::Session>),
    L625(Box<pal::pipeline::Session>),
}

macro_rules! on_deck {
    ($deck:expr, $s:ident => $body:expr) => {
        match $deck {
            Deck::L525($s) => $body,
            Deck::L625($s) => $body,
        }
    };
}

impl Deck {
    fn new(params: StageParams, start: i64) -> Self {
        match params {
            StageParams::L525(p) => Deck::L525(Box::new(ntsc::pipeline::Session::with_start(*p, start))),
            StageParams::L625(p) => Deck::L625(Box::new(pal::pipeline::Session::with_start(*p, start))),
        }
    }

    fn raster(&self) -> Raster {
        match self {
            Deck::L525(_) => Raster::L525,
            Deck::L625(_) => Raster::L625,
        }
    }

    fn push_frame(&mut self, f: &Frame) {
        on_deck!(self, s => s.push(f))
    }

    fn finish(&mut self) {
        on_deck!(self, s => s.finish())
    }

    fn ready(&self) -> i64 {
        on_deck!(self, s => s.ready())
    }

    fn process(&mut self, max: usize) -> Vec<Frame> {
        on_deck!(self, s => s.process(max))
    }

    fn set_automation(&mut self, schedule: Option<Arc<AutomationSchedule>>) {
        on_deck!(self, s => s.automation = schedule)
    }

    /// Play back up to `max` frames as tagged chunks of the deck's own raster.
    fn pull(&mut self, max: usize) -> Vec<SignalChunk> {
        let raster = self.raster();
        let tag = |k0: i64, planes: Vec<Planes>| planes.into_iter().enumerate().map(|(i, p)| SignalChunk::new(raster, k0 + i as i64, p)).collect();
        match self {
            Deck::L525(s) => {
                let (k0, bbs) = s.process_baseband(max);
                tag(k0, bbs.into_iter().map(|b| Planes { y: b.y, cu: b.cu, cv: b.cv }).collect())
            }
            Deck::L625(s) => {
                let (k0, bbs) = s.process_baseband(max);
                tag(k0, bbs.into_iter().map(|b| Planes { y: b.y, cu: b.cu, cv: b.cv }).collect())
            }
        }
    }
}

/// A deck's record input: it takes the next frame of its own raster and nothing else.
struct RecordInput {
    deck: Deck,
    next: i64,
}

impl RecordInput {
    fn push(&mut self, chunk: SignalChunk) -> Result<(), String> {
        let raster = self.deck.raster();
        if chunk.port.raster != raster {
            return Err(format!("a {} deck cannot record a {} signal", raster.name(), chunk.port.raster.name()));
        }
        chunk.check()?;
        if chunk.frame != self.next {
            return Err(format!("the deck expects frame {}, the signal carries frame {}", self.next, chunk.frame));
        }
        self.next += 1;
        let Planes { y, cu, cv } = chunk.planes;
        match &mut self.deck {
            Deck::L525(s) => s.push_baseband(ntsc::video::Baseband { y, cu, cv }),
            Deck::L625(s) => s.push_baseband(pal::video::Baseband { y, cu, cv }),
        }
        Ok(())
    }
}

// ------------------------------------------------------------------ links

/// A link at work.
enum Link {
    /// Direct reference and separate Y/C: the samples pass unchanged.
    Through,
    Composite { separator: Separator, system: CompositeSystem },
    Converter(Box<StandardsConverter>),
}

/// Encode a frame on the subcarrier of `system` and separate it again, on its own engine.
fn composite_hop(chunk: &mut SignalChunk, separator: Separator, system: CompositeSystem) {
    macro_rules! hop {
        ($engine:ident) => {{
            use $engine::composite::{roundtrip_with_tv, CompositeDecoder, CompositeParams};
            let decoder = match separator {
                Separator::Notch => CompositeDecoder::Notch,
                Separator::Comb1h => CompositeDecoder::Comb1h,
                Separator::Comb2h => CompositeDecoder::Comb2h,
            };
            let tv = match system.colour() {
                crate::signal::ColourSystem::Ntsc => crate::standard::ColourTv::Ntsc,
                crate::signal::ColourSystem::Pal => crate::standard::ColourTv::Pal,
            };
            let planes = std::mem::replace(&mut chunk.planes, Planes { y: Vec::new(), cu: Vec::new(), cv: Vec::new() });
            let mut bb = $engine::video::Baseband { y: planes.y, cu: planes.cu, cv: planes.cv };
            roundtrip_with_tv(&mut bb, chunk.frame, &CompositeParams { enabled: true, decoder }, tv);
            chunk.planes = Planes { y: bb.y, cu: bb.cu, cv: bb.cv };
        }};
    }
    match chunk.port.raster {
        Raster::L525 => hop!(ntsc),
        Raster::L625 => hop!(pal),
    }
}

impl Link {
    fn transfer(&mut self, mut chunk: SignalChunk) -> Result<Vec<SignalChunk>, String> {
        match self {
            Link::Through => Ok(vec![chunk]),
            Link::Composite { separator, system } => {
                composite_hop(&mut chunk, *separator, *system);
                Ok(vec![chunk])
            }
            Link::Converter(c) => {
                c.push(chunk)?;
                Ok(c.pull())
            }
        }
    }

    /// The input has ended, and the programme with it at `end` seconds: what is still to come.
    fn finish(&mut self, end: Rational) -> Vec<SignalChunk> {
        match self {
            Link::Converter(c) => {
                c.finish_at(end);
                c.pull()
            }
            _ => Vec::new(),
        }
    }

    fn timing(&self) -> LinkTiming {
        match self {
            Link::Converter(c) => c.timing(),
            _ => LinkTiming::NONE,
        }
    }
}

// ------------------------------------------------------------------ the record of a render

#[derive(Debug, Serialize)]
pub struct StageRecord {
    pub name: String,
    pub standard: &'static str,
    pub raster: &'static str,
    pub seed: u64,
    pub first_frame: i64,
    /// Programme time of the first frame, seconds, as an exact fraction.
    pub first_frame_time: String,
}

#[derive(Debug, Serialize)]
pub struct LinkRecord {
    pub name: String,
    pub link_type: &'static str,
    pub output_port: String,
    pub input_port: String,
    pub signal_delay_s: String,
    pub lookahead_s: String,
    /// The sound takes the line connection beside the picture: no delay and no retiming.
    pub audio_route: &'static str,
    pub converter: Option<ConverterRecord>,
}

/// What was built: every generation, every link, every clock.
#[derive(Debug, Serialize)]
pub struct RackRecord {
    pub generations: Vec<StageRecord>,
    pub links: Vec<LinkRecord>,
}

impl RackRecord {
    /// One line per generation and link, for a person.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for (i, g) in self.generations.iter().enumerate() {
            out.push_str(&format!("  generation {i} '{}': {} on {}, seed {}, from frame {} ({} s)\n", g.name, g.standard, g.raster, g.seed, g.first_frame, g.first_frame_time));
            if let Some(l) = self.links.get(i) {
                let detail = match &l.converter {
                    Some(c) => format!(", profile {} (lookahead {} s, not calibrated against hardware)", c.profile, c.lookahead_s),
                    None => String::new(),
                };
                out.push_str(&format!("    link '{}': {} [{} -> {}]{detail}\n", l.name, l.link_type, l.output_port, l.input_port));
            }
        }
        out
    }
}

// ------------------------------------------------------------------ the rack

/// Generations of either raster in series. Driven like a `Chain`: push source frames, take
/// output frames while `ready`, `finish` at the end of the input.
pub struct Rack {
    stages: Vec<RecordInput>,
    links: Vec<Link>,
    record: RackRecord,
    /// The whole schedule, as given to `set_automation`.
    pub automation: Option<Arc<AutomationSchedule>>,
}

impl Rack {
    /// Build the rack for a programme that starts at time `start`, seconds. The first
    /// generation starts at its first frame at or after `start`, each later one at its first
    /// frame at or after the start of the one before, so no generation is asked for a picture
    /// from before its source. The tapes are functions of the absolute frame index, as in
    /// `Session::with_start`.
    pub fn new(plan: RackPlan, start: Rational) -> Result<Self, String> {
        let ports = plan.validate()?;
        let mut starts = Vec::with_capacity(plan.stages.len());
        let mut t = start;
        for s in &plan.stages {
            let raster = s.params.raster();
            let k = raster.first_frame_at_or_after(t);
            t = raster.frame_time(k);
            starts.push(k);
        }
        let mut links = Vec::with_capacity(plan.links.len());
        let mut link_records = Vec::with_capacity(plan.links.len());
        for (i, link) in plan.links.iter().enumerate() {
            let (up, down) = (&plan.stages[i].params, &plan.stages[i + 1].params);
            let made = match link.spec {
                LinkSpec::DirectYcReference | LinkSpec::SeparateYc => Link::Through,
                LinkSpec::Composite { separator } => Link::Composite { separator, system: up.output_system() },
                LinkSpec::StandardsConverter { profile } => Link::Converter(Box::new(
                    StandardsConverter::new(profile, up.raster(), down.raster(), starts[i], starts[i + 1]).map_err(|e| format!("link {i} '{}': {e}", link.name))?,
                )),
            };
            let timing = made.timing();
            link_records.push(LinkRecord {
                name: link.name.clone(),
                link_type: link.spec.type_name(),
                output_port: ports[i].0.describe(),
                input_port: ports[i].1.describe(),
                signal_delay_s: timing.signal_delay.to_string(),
                lookahead_s: timing.lookahead.to_string(),
                audio_route: "line, unchanged",
                converter: match &made {
                    Link::Converter(c) => Some(c.record()),
                    _ => None,
                },
            });
            links.push(made);
        }
        let generations = plan
            .stages
            .iter()
            .zip(&starts)
            .map(|(s, &k)| StageRecord {
                name: s.name.clone(),
                standard: s.params.standard().name(),
                raster: s.params.raster().name(),
                seed: s.params.seed(),
                first_frame: k,
                first_frame_time: s.params.raster().frame_time(k).to_string(),
            })
            .collect();
        let stages = plan.stages.into_iter().zip(&starts).map(|(s, &k)| RecordInput { deck: Deck::new(s.params, k), next: k }).collect();
        Ok(Self { stages, links, record: RackRecord { generations, links: link_records }, automation: None })
    }

    /// Raster of the frames `push` takes (the first generation's).
    pub fn input_raster(&self) -> Raster {
        self.stages[0].deck.raster()
    }

    /// Raster of the frames `process` gives (the last generation's).
    pub fn output_raster(&self) -> Raster {
        self.stages[self.stages.len() - 1].deck.raster()
    }

    /// Frame index of the first frame `push` takes, on the input raster's clock.
    pub fn input_start_frame(&self) -> i64 {
        self.record.generations[0].first_frame
    }

    /// Programme time of the first frame `push` takes, seconds.
    pub fn input_start_time(&self) -> Rational {
        self.input_raster().frame_time(self.input_start_frame())
    }

    /// Programme time of the first output frame, seconds. Sound that is to play with the
    /// picture starts here.
    pub fn output_start_time(&self) -> Rational {
        let last = self.record.generations.len() - 1;
        self.output_raster().frame_time(self.record.generations[last].first_frame)
    }

    pub fn record(&self) -> &RackRecord {
        &self.record
    }

    /// As `Chain::set_automation`: the first generation takes the camera's tracks, the last
    /// the whole schedule, those between none; a single generation takes it all. Keyframe
    /// positions are frames of the generation that reads them.
    pub fn set_automation(&mut self, schedule: Option<AutomationSchedule>) {
        let whole = schedule.map(Arc::new);
        self.automation = whole.clone();
        let camera = whole.as_deref().map(|s| s.subset(ntsc::pipeline::CAMERA_TRACKS)).filter(|s| !s.tracks.is_empty()).map(Arc::new);
        let last = self.stages.len() - 1;
        for (i, stage) in self.stages.iter_mut().enumerate() {
            stage.deck.set_automation(if i == last {
                whole.clone()
            } else if i == 0 {
                camera.clone()
            } else {
                None
            });
        }
    }

    pub fn with_automation(mut self, schedule: AutomationSchedule) -> Self {
        self.set_automation(Some(schedule));
        self
    }

    /// Hand what generation `i` can play back to generation `i + 1` through their link:
    /// whole batches, or with `all` everything there is.
    fn forward(&mut self, i: usize, all: bool) {
        loop {
            let ready = self.stages[i].deck.ready();
            if ready == 0 || (!all && ready < FORWARD_BATCH as i64) {
                return;
            }
            for chunk in self.stages[i].deck.pull(FORWARD_BATCH) {
                // the edges were validated when the rack was built
                let through = self.links[i].transfer(chunk).expect("a validated link refused its signal");
                for c in through {
                    self.stages[i + 1].push(c).expect("a validated record input refused its signal");
                }
            }
        }
    }

    /// Queue a source frame for the first generation (the active picture of the input raster).
    pub fn push(&mut self, f: &Frame) {
        self.stages[0].deck.push_frame(f);
        self.stages[0].next += 1;
        for i in 0..self.links.len() {
            self.forward(i, false);
        }
    }

    /// No more source frames: finish each generation in turn, play all of it into its link,
    /// flush the link once, and go on to the next.
    pub fn finish(&mut self) {
        // the programme is the source frames that were pushed; no link plays past its end
        let end = self.input_raster().frame_time(self.stages[0].next);
        for i in 0..self.stages.len() {
            self.stages[i].deck.finish();
            if i < self.links.len() {
                self.forward(i, true);
                for c in self.links[i].finish(end) {
                    self.stages[i + 1].push(c).expect("a validated record input refused its signal");
                }
            }
        }
    }

    /// Output frames that can be produced right now.
    pub fn ready(&self) -> i64 {
        self.stages[self.stages.len() - 1].deck.ready()
    }

    /// Produce up to `max_frames` output frames. Only here does a signal meet the picture tube.
    pub fn process(&mut self, max_frames: usize) -> Vec<Frame> {
        let last = self.stages.len() - 1;
        self.stages[last].deck.process(max_frames)
    }

    /// Convenience: run a whole clip from programme time zero.
    pub fn run(plan: RackPlan, input: &[Frame]) -> Result<Vec<Frame>, String> {
        let mut rack = Rack::new(plan, Rational::ZERO)?;
        let mut out = Vec::new();
        for f in input {
            rack.push(f);
            while rack.ready() > 0 {
                out.extend(rack.process(4));
            }
        }
        rack.finish();
        while rack.ready() > 0 {
            out.extend(rack.process(4));
        }
        Ok(out)
    }
}

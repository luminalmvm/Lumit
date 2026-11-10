//! The check: what won't come across in a vector export, layer by layer, and
//! which players miss what.
//!
//! # In plain terms
//!
//! A Lottie or an SVG can carry most of a motion-graphics comp but not all of
//! it, and the players that draw a Lottie don't all draw the same things. The
//! export never stops for that: it writes what it can and says, for each
//! layer, what it left out, what only some players draw, and what it changed
//! on the way (baked an expression into keys, flattened a 3D layer). The
//! window lists these before anything is written, so they can be fixed first.
//!
//! An issue carries a stable reason id and its facts, never a sentence: the
//! interface translates by id (`engine_labels.dart`), with [`Issue::english`]
//! as the fallback for an id it has no sentence for yet.
//!
//! Runs on the caller's thread with the rest of the export.

use uuid::Uuid;

/// A set of players, one bit each. The check is judged against the ones the
/// person ticked; the bytes written never depend on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Players(pub u16);

impl Players {
    /// lottie-web's SVG renderer, what most pages use.
    pub const WEB: Players = Players(1);
    /// lottie-web's canvas renderer.
    pub const WEB_CANVAS: Players = Players(1 << 1);
    /// The dotLottie players (ThorVG): web, iOS and Android.
    pub const DOT_LOTTIE: Players = Players(1 << 2);
    /// lottie-ios.
    pub const IOS: Players = Players(1 << 3);
    /// lottie-android.
    pub const ANDROID: Players = Players(1 << 4);
    /// The Flutter `lottie` package, which the preview plays through.
    pub const FLUTTER: Players = Players(1 << 5);
    /// Every Lottie player above.
    pub const ALL: Players = Players(0b11_1111);
    /// Browsers playing an animated SVG.
    pub const CHROME: Players = Players(1 << 8);
    pub const FIREFOX: Players = Players(1 << 9);
    pub const SAFARI: Players = Players(1 << 10);
    /// Every browser above.
    pub const BROWSERS: Players = Players(0b111 << 8);
    pub const NONE: Players = Players(0);

    /// Every player, by the key the bridge and the command line name it, in
    /// the order the window lists them.
    pub const KEYS: [(&'static str, Players); 9] = [
        ("web", Players::WEB),
        ("web_canvas", Players::WEB_CANVAS),
        ("dotlottie", Players::DOT_LOTTIE),
        ("ios", Players::IOS),
        ("android", Players::ANDROID),
        ("flutter", Players::FLUTTER),
        ("chrome", Players::CHROME),
        ("firefox", Players::FIREFOX),
        ("safari", Players::SAFARI),
    ];

    #[must_use]
    pub fn contains(self, other: Players) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub fn and(self, other: Players) -> Players {
        Players(self.0 & other.0)
    }

    #[must_use]
    pub fn or(self, other: Players) -> Players {
        Players(self.0 | other.0)
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The set named by `keys`; an unknown key is skipped.
    #[must_use]
    pub fn from_keys<'a>(keys: impl IntoIterator<Item = &'a str>) -> Players {
        keys.into_iter().fold(Players::NONE, |set, key| {
            Players::KEYS
                .iter()
                .find(|(k, _)| *k == key)
                .map_or(set, |(_, p)| set.or(*p))
        })
    }

    /// The keys of the players in this set, in [`Self::KEYS`] order.
    #[must_use]
    pub fn keys(self) -> Vec<&'static str> {
        Players::KEYS
            .iter()
            .filter(|(_, p)| self.contains(*p))
            .map(|(k, _)| *k)
            .collect()
    }
}

/// What happens to the thing an issue is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Outcome {
    /// Not in the file at all, or in it and drawn by none of the ticked players.
    LeftOut,
    /// In the file, and some of the ticked players don't draw it.
    SomePlayers,
    /// In the file, written differently: baked into keys, drawn flat.
    Changed,
}

impl Outcome {
    /// The id the bridge sends.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Outcome::LeftOut => "left_out",
            Outcome::SomePlayers => "some_players",
            Outcome::Changed => "changed",
        }
    }
}

/// One thing that won't come across as it is in Lumit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Issue {
    pub outcome: Outcome,
    /// The stable id the interface translates by, such as `effect`.
    pub reason: &'static str,
    /// The facts the sentence is built from, by name: `label` → `Glow`.
    pub args: Vec<(String, String)>,
    /// The English sentence, for an interface with no translation of the id.
    pub english: String,
    /// The comp the layer is in, which is a precomp's own comp for a layer
    /// inside one.
    pub comp: Uuid,
    /// The layer, when the issue is about one.
    pub layer: Option<Uuid>,
    /// Where it is, for the reader: comp, layer, then the part of the layer.
    pub place: Vec<String>,
    /// The ticked players that miss it. Empty for [`Outcome::LeftOut`] and
    /// [`Outcome::Changed`].
    pub players: Players,
}

/// A feature some players draw and some don't. Each row of [`missing_in`]
/// comes from research part 3's player table, read from each player's own
/// source and changelog (lottie-web 5.13, lottie-android 6.7, lottie-ios
/// master, Flutter lottie 3.8, ThorVG main).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    /// A blend mode, by its Lottie code (1 multiply .. 17 hard mix).
    BlendMode(u8),
    MaskSubtract,
    MaskIntersect,
    /// Lighten and Darken masks.
    MaskLightenDarken,
    MaskDifference,
    /// A mask's opacity below 100.
    MaskOpacity,
    MaskExpansion,
    LumaMatte,
    OffsetPaths,
    /// Per-dimension easing on a value with several numbers.
    SplitEasing,
    /// Rounded corners on a polygon or star.
    RoundedPolystar,
    /// Text drawn from a font (Text mode).
    FontText,
    TextAnimators,
    TextOnPath,
    EffectTint,
    EffectFill,
    EffectTritone,
    EffectDropShadow,
    EffectBlur,
    EffectStroke,
    EffectLevels,
    EffectTransform,
    /// Set matte (Lottie effect 28).
    EffectSetMatte,
    /// `mix-blend-mode` in an SVG.
    SvgBlend,
}

/// The players in `ticked` that don't draw `feature`.
///
/// A cell the research left unconfirmed counts as missing: a warning that
/// turns out unneeded costs a glance, and a missing one costs a broken
/// animation on someone's phone.
#[must_use]
pub fn missing_in(feature: Feature, ticked: Players) -> Players {
    const MOBILE: Players = Players(Players::IOS.0 | Players::ANDROID.0 | Players::FLUTTER.0);
    let missing = match feature {
        // Multiply, screen, overlay, darken, lighten: Android draws these and
        // add; lottie-web and the canvas renderer draw 1 to 15; Flutter 1 to
        // 16; ThorVG all. iOS parses them and its drawing is unconfirmed.
        Feature::BlendMode(1..=5) => Players::IOS,
        Feature::BlendMode(6..=15) => Players::IOS.or(Players::ANDROID),
        Feature::BlendMode(16) => Players::WEB.or(Players::WEB_CANVAS).or(Players::IOS),
        Feature::BlendMode(_) => Players::WEB.or(Players::WEB_CANVAS).or(MOBILE),
        // The canvas renderer ORs every mask into one clip.
        Feature::MaskSubtract | Feature::MaskOpacity => Players::WEB_CANVAS,
        Feature::MaskIntersect => Players::WEB_CANVAS.or(Players::FLUTTER),
        Feature::MaskLightenDarken => Players::WEB.or(Players::WEB_CANVAS).or(MOBILE),
        Feature::MaskDifference => Players::WEB.or(Players::WEB_CANVAS).or(MOBILE),
        Feature::MaskExpansion => Players::WEB_CANVAS.or(MOBILE),
        // Android and Flutter draw a luma matte as an alpha one; iOS has none.
        Feature::LumaMatte => MOBILE,
        Feature::OffsetPaths => MOBILE,
        // Per-dimension handles are read by lottie-web and ThorVG; the mobile
        // players' reading of them is unconfirmed.
        Feature::SplitEasing => MOBILE,
        Feature::RoundedPolystar => Players::IOS,
        Feature::FontText => Players::NONE,
        Feature::TextAnimators => MOBILE,
        Feature::TextOnPath => MOBILE,
        Feature::EffectTint | Feature::EffectFill | Feature::EffectTritone => {
            Players::WEB_CANVAS.or(MOBILE)
        }
        Feature::EffectDropShadow => Players::WEB_CANVAS,
        Feature::EffectBlur => Players::WEB_CANVAS.or(Players::IOS),
        // lottie-web's SVG renderer draws all three. The canvas renderer
        // draws only Transform, ThorVG only Stroke, and no mobile player any.
        Feature::EffectStroke => Players::WEB_CANVAS.or(MOBILE),
        Feature::EffectLevels => Players::WEB_CANVAS.or(Players::DOT_LOTTIE).or(MOBILE),
        Feature::EffectTransform => Players::DOT_LOTTIE.or(MOBILE),
        // lottie-web's SVG renderer alone (`SVGMatte3Effect`). Its canvas
        // renderer draws only Transform, Android and Flutter only blur and
        // shadow, and it is unconfirmed in ThorVG and iOS.
        Feature::EffectSetMatte => Players::WEB_CANVAS.or(Players::DOT_LOTTIE).or(MOBILE),
        Feature::SvgBlend => Players::SAFARI,
    };
    missing.and(ticked)
}

/// Builds the issue list as the export walks the comp, and settles each
/// issue's outcome against the ticked players.
#[derive(Debug, Default)]
pub struct Report {
    issues: Vec<Issue>,
    /// The issues already said, for finding a repeat without a scan.
    said: std::collections::BTreeSet<Issue>,
    ticked: Players,
}

impl Report {
    #[must_use]
    pub fn new(ticked: Players) -> Self {
        Self {
            issues: Vec::new(),
            said: std::collections::BTreeSet::new(),
            ticked,
        }
    }

    /// Something the file can't carry at all.
    pub fn left_out(&mut self, at: &Place, reason: &'static str, english: String, args: Args) {
        self.push(at, Outcome::LeftOut, reason, english, args, Players::NONE);
    }

    /// Something written differently from how Lumit has it.
    pub fn changed(&mut self, at: &Place, reason: &'static str, english: String, args: Args) {
        self.push(at, Outcome::Changed, reason, english, args, Players::NONE);
    }

    /// Something only some players draw. Nothing is said when every ticked
    /// player draws it, and it counts as left out when none of them does.
    pub fn players(
        &mut self,
        at: &Place,
        feature: Feature,
        reason: &'static str,
        english: String,
        args: Args,
    ) {
        let missing = missing_in(feature, self.ticked);
        if missing.is_empty() {
            return;
        }
        let outcome = if missing == self.ticked {
            Outcome::LeftOut
        } else {
            Outcome::SomePlayers
        };
        self.push(at, outcome, reason, english, args, missing);
    }

    fn push(
        &mut self,
        at: &Place,
        outcome: Outcome,
        reason: &'static str,
        english: String,
        args: Args,
        players: Players,
    ) {
        let issue = Issue {
            outcome,
            reason,
            args,
            english,
            comp: at.comp,
            layer: at.layer,
            place: at.names.clone(),
            players,
        };
        // The same thing said twice about the same place is said once: a
        // layer with two blurs has one line about blurs.
        if self.said.insert(issue.clone()) {
            self.issues.push(issue);
        }
    }

    /// The issues, left out first, in the order they were found within each.
    #[must_use]
    pub fn finish(mut self) -> Vec<Issue> {
        self.issues.sort_by_key(|issue| issue.outcome);
        self.issues
    }
}

/// The facts of an issue, by name.
pub type Args = Vec<(String, String)>;

/// Where the walk is: which comp and layer, and the names to show.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub comp: Uuid,
    pub layer: Option<Uuid>,
    pub names: Vec<String>,
}

impl Place {
    /// The comp itself.
    #[must_use]
    pub fn comp(comp: Uuid, name: &str) -> Self {
        Self {
            comp,
            layer: None,
            names: vec![name.to_owned()],
        }
    }

    /// A layer in this place's comp.
    #[must_use]
    pub fn layer(&self, layer: Uuid, name: &str) -> Self {
        let mut names = self.names.clone();
        names.push(name.to_owned());
        Self {
            comp: self.comp,
            layer: Some(layer),
            names,
        }
    }

    /// A part of this place's layer: an effect, a mask, a shape item.
    #[must_use]
    pub fn part(&self, name: &str) -> Self {
        let mut names = self.names.clone();
        names.push(name.to_owned());
        Self {
            comp: self.comp,
            layer: self.layer,
            names,
        }
    }

    /// A precomp's own comp, entered through a layer here: issues inside it
    /// are about its own layers, and keep the road in for the reader.
    #[must_use]
    pub fn inside(&self, comp: Uuid) -> Self {
        Self {
            comp,
            layer: None,
            names: self.names.clone(),
        }
    }
}

/// One fact for an issue.
#[must_use]
pub fn arg(name: &str, value: impl Into<String>) -> (String, String) {
    (name.to_owned(), value.into())
}

//! Full Q5 Vorbis setup parser: codebooks, floors, residues, mappings, modes.
//!
//! Reads a single Vorbis setup-header blob (the bytes starting with `\x05vorbis`)
//! and returns a `Q5Setup` struct with everything the encoder needs at runtime.

#![allow(clippy::needless_range_loop)]

use crate::bitpack::BitReader;
use crate::codebook::{unpack_codebook, Codebook};
use crate::floor1::{floor1_look, unpack_floor1, Floor1State};
use crate::psy::VorbisInfoMapping0;
use crate::residue::{residue_look, unpack_residue, ResidueLook, ResidueSetup};

// ---------------------------------------------------------------------------
// ov_ilog: integer log2
// ---------------------------------------------------------------------------

fn ov_ilog(mut v: u32) -> u32 {
    let mut ret = 0u32;
    while v != 0 {
        ret += 1;
        v >>= 1;
    }
    ret
}

// ---------------------------------------------------------------------------
// Mapping struct
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct Mapping {
    pub submaps: usize,
    pub chmuxlist: Vec<usize>,
    pub floorsubmap: Vec<usize>,
    pub residuesubmap: Vec<usize>,
    #[allow(dead_code)]
    pub coupling_steps: usize,
    #[allow(dead_code)]
    pub coupling_mag: Vec<usize>,
    #[allow(dead_code)]
    pub coupling_ang: Vec<usize>,
    /// Mirrors VorbisInfoMapping0 for use in vp_couple_quantize_normalize
    pub vp_mapping: VorbisInfoMapping0,
}

// ---------------------------------------------------------------------------
// Mode struct (mirrors vorbis_info_mode)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct Mode {
    pub blockflag: bool,
    pub mapping: usize,
}

// ---------------------------------------------------------------------------
// Q5Setup: everything the encoder needs
// ---------------------------------------------------------------------------

pub(crate) struct Q5Setup {
    #[allow(dead_code)]
    pub channels: usize,
    pub books: Vec<Codebook>,
    pub floor_states: Vec<Floor1State>,
    /// Residue type (0, 1, or 2) + setup + look for each residue
    pub residue_types: Vec<u16>,
    pub residue_setups: Vec<ResidueSetup>,
    pub residue_looks: Vec<ResidueLook>,
    pub mappings: Vec<Mapping>,
    pub modes: Vec<Mode>,
    pub modebits: u32,
}

// ---------------------------------------------------------------------------
// Parse error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) enum SetupError {
    BadMagic,
    Codebook(#[allow(dead_code)] crate::codebook::CodebookError),
    Floor(#[allow(dead_code)] crate::floor1::Floor1Error),
    Residue(#[allow(dead_code)] crate::residue::ResidueError),
    BadMappingType,
    #[allow(dead_code)]
    BadChannels,
    BadModeWindowtype,
    BadBitstream,
}

impl From<crate::codebook::CodebookError> for SetupError {
    fn from(e: crate::codebook::CodebookError) -> Self {
        SetupError::Codebook(e)
    }
}
impl From<crate::floor1::Floor1Error> for SetupError {
    fn from(e: crate::floor1::Floor1Error) -> Self {
        SetupError::Floor(e)
    }
}
impl From<crate::residue::ResidueError> for SetupError {
    fn from(e: crate::residue::ResidueError) -> Self {
        SetupError::Residue(e)
    }
}

// ---------------------------------------------------------------------------
// unpack_q5_setup: parse the full setup blob
// ---------------------------------------------------------------------------

pub(crate) fn unpack_q5_setup(blob: &[u8], channels: usize) -> Result<Q5Setup, SetupError> {
    if blob.len() < 7 || &blob[0..7] != b"\x05vorbis" {
        return Err(SetupError::BadMagic);
    }
    let mut r = BitReader::new(&blob[7..]);

    // --- Codebooks ---
    let books_count = r.read(8) as usize + 1;
    let mut books = Vec::with_capacity(books_count);
    for _ in 0..books_count {
        books.push(unpack_codebook(&mut r)?);
    }

    // --- Time domain placeholders ---
    let times = r.read(6) as usize + 1;
    for _ in 0..times {
        let _t = r.read(16);
    }

    // --- Floors ---
    let floors_count = r.read(6) as usize + 1;
    let mut floor_setups = Vec::with_capacity(floors_count);
    for _ in 0..floors_count {
        let floor_type = r.read(16);
        if floor_type != 1 {
            return Err(SetupError::BadBitstream);
        }
        let mut setup = unpack_floor1(&mut r, books_count)?;
        // Wire up Floor1Setup encode-side fields for Q5 long block (n=2048).
        // Values from libvorbis lib/modes/floor_all.h, floor template #8 (2048x27):
        //   maxover=60, maxunder=30, maxerr=500, twofitweight=3, twofitatten=18, n=2048
        setup.maxover = 60.0;
        setup.maxunder = 30.0;
        setup.maxerr = 500.0;
        setup.twofitweight = 3.0;
        setup.twofitatten = 18.0;
        setup.n = setup.postlist[1]; // n = the block half-size for this floor
        floor_setups.push(setup);
    }

    // Build floor states (look structs)
    let floor_states: Vec<Floor1State> = floor_setups.into_iter().map(floor1_look).collect();

    // --- Residues ---
    let residues_count = r.read(6) as usize + 1;
    let mut residue_types = Vec::with_capacity(residues_count);
    let mut residue_setups = Vec::with_capacity(residues_count);
    for _ in 0..residues_count {
        let residue_type = r.read(16) as u16;
        let setup = unpack_residue(&mut r, books_count)?;
        residue_types.push(residue_type);
        residue_setups.push(setup);
    }

    // Build residue looks
    let residue_looks: Vec<ResidueLook> = residue_setups
        .iter()
        .map(|s| residue_look(s, &books))
        .collect();

    // --- Mappings ---
    let mappings_count = r.read(6) as usize + 1;
    let mut mappings = Vec::with_capacity(mappings_count);
    for _ in 0..mappings_count {
        let mapping_type = r.read(16);
        if mapping_type != 0 {
            return Err(SetupError::BadMappingType);
        }
        let mapping = unpack_mapping(&mut r, channels, floors_count, residues_count)?;
        mappings.push(mapping);
    }

    // --- Modes ---
    let modes_count = r.read(6) as usize + 1;
    let mut modes = Vec::with_capacity(modes_count);
    for _ in 0..modes_count {
        let blockflag = r.read(1) != 0;
        let windowtype = r.read(16);
        let transformtype = r.read(16);
        if windowtype != 0 || transformtype != 0 {
            return Err(SetupError::BadModeWindowtype);
        }
        let mapping = r.read(8) as usize;
        modes.push(Mode { blockflag, mapping });
    }

    // modebits: number of bits needed to encode mode number
    let modebits = ov_ilog(modes_count as u32 - 1);

    Ok(Q5Setup {
        channels,
        books,
        floor_states,
        residue_types,
        residue_setups,
        residue_looks,
        mappings,
        modes,
        modebits,
    })
}

fn unpack_mapping(
    r: &mut BitReader,
    channels: usize,
    floors_count: usize,
    residues_count: usize,
) -> Result<Mapping, SetupError> {
    let has_submaps = r.read(1) != 0;
    let submaps = if has_submaps {
        r.read(4) as usize + 1
    } else {
        1
    };

    let has_coupling = r.read(1) != 0;
    let coupling_steps;
    let mut coupling_mag = Vec::new();
    let mut coupling_ang = Vec::new();

    if has_coupling {
        coupling_steps = r.read(8) as usize + 1;
        let ch_bits = ov_ilog(channels as u32 - 1);
        for _ in 0..coupling_steps {
            let mag = r.read(ch_bits) as usize;
            let ang = r.read(ch_bits) as usize;
            coupling_mag.push(mag);
            coupling_ang.push(ang);
        }
    } else {
        coupling_steps = 0;
    }

    // reserved 2 bits
    let _reserved = r.read(2);

    let mut chmuxlist = vec![0usize; channels];
    if submaps > 1 {
        for i in 0..channels {
            chmuxlist[i] = r.read(4) as usize;
        }
    }

    let mut floorsubmap = vec![0usize; submaps];
    let mut residuesubmap = vec![0usize; submaps];
    for i in 0..submaps {
        let _time_config = r.read(8); // unused
        let fs = r.read(8) as usize;
        let rs = r.read(8) as usize;
        if fs >= floors_count || rs >= residues_count {
            return Err(SetupError::BadBitstream);
        }
        floorsubmap[i] = fs;
        residuesubmap[i] = rs;
    }

    let vp_mapping = VorbisInfoMapping0 {
        coupling_steps,
        coupling_mag: coupling_mag.clone(),
        coupling_ang: coupling_ang.clone(),
    };

    Ok(Mapping {
        submaps,
        chmuxlist,
        floorsubmap,
        residuesubmap,
        coupling_steps,
        coupling_mag,
        coupling_ang,
        vp_mapping,
    })
}

// ---------------------------------------------------------------------------
// OnceLock-cached Q5 setups for each (rate, channels) combo
// ---------------------------------------------------------------------------

use std::sync::OnceLock;

use crate::{Channels, SampleRate};

pub(crate) fn q5_setup_for(rate: SampleRate, channels: Channels) -> &'static Q5Setup {
    use crate::setup_blob::{
        Q5_SETUP_MONO44, Q5_SETUP_MONO48, Q5_SETUP_STEREO44, Q5_SETUP_STEREO48,
    };

    match (rate, channels) {
        (SampleRate::Hz44100, Channels::Mono) => {
            static CACHE: OnceLock<Q5Setup> = OnceLock::new();
            CACHE.get_or_init(|| {
                unpack_q5_setup(Q5_SETUP_MONO44, 1).expect("mono44 setup parse failed")
            })
        }
        (SampleRate::Hz48000, Channels::Mono) => {
            static CACHE: OnceLock<Q5Setup> = OnceLock::new();
            CACHE.get_or_init(|| {
                unpack_q5_setup(Q5_SETUP_MONO48, 1).expect("mono48 setup parse failed")
            })
        }
        (SampleRate::Hz44100, Channels::Stereo) => {
            static CACHE: OnceLock<Q5Setup> = OnceLock::new();
            CACHE.get_or_init(|| {
                unpack_q5_setup(Q5_SETUP_STEREO44, 2).expect("stereo44 setup parse failed")
            })
        }
        (SampleRate::Hz48000, Channels::Stereo) => {
            static CACHE: OnceLock<Q5Setup> = OnceLock::new();
            CACHE.get_or_init(|| {
                unpack_q5_setup(Q5_SETUP_STEREO48, 2).expect("stereo48 setup parse failed")
            })
        }
    }
}

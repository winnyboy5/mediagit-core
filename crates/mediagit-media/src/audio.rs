// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

//! Audio track analysis and merging
//!
//! This module provides audio file parsing and multi-track analysis
//! to enable intelligent merging of non-overlapping audio edits.
//!
//! # Features
//!
//! - Multi-format audio parsing (MP3, WAV, FLAC, AAC, OGG)
//! - Track detection and identification
//! - Auto-merge different tracks (e.g., music + vocals)
//! - Conflict detection for same-track modifications
//! - Waveform metadata extraction
//!
//! # Example
//!
//! ```rust,no_run
//! use mediagit_media::audio::AudioParser;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let audio_data = std::fs::read("audio.mp3")?;
//! let parser = AudioParser::new();
//! let info = parser.parse(&audio_data, "audio.mp3").await?;
//!
//! println!("Duration: {:.2} seconds", info.duration_seconds);
//! println!("Channels: {}", info.channels);
//! # Ok(())
//! # }
//! ```

use crate::error::{MediaError, Result};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tracing::{debug, info, instrument, warn};

/// Complete audio file information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioInfo {
    /// Audio duration in seconds
    pub duration_seconds: f64,

    /// Sample rate (Hz)
    pub sample_rate: u32,

    /// Number of audio channels
    pub channels: u16,

    /// Bits per sample
    pub bit_depth: Option<u16>,

    /// Audio codec
    pub codec: String,

    /// Bitrate (bits per second)
    pub bitrate: Option<u32>,

    /// All tracks in the audio (for multi-track formats)
    pub tracks: Vec<AudioTrack>,

    /// Audio format
    pub format: AudioFormat,
}

/// Information about a single audio track
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioTrack {
    /// Track ID
    pub id: u32,

    /// Track name/title
    pub name: Option<String>,

    /// Track type (vocals, music, effects, etc.)
    pub track_type: TrackType,

    /// Track duration in seconds
    pub duration_seconds: f64,

    /// Track-specific sample rate
    pub sample_rate: u32,

    /// Track-specific channels
    pub channels: u16,

    /// Start time in the composition
    pub start_time: f64,
}

/// Type of audio track
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackType {
    /// Vocal track
    Vocals,
    /// Music/instrumental track
    Music,
    /// Sound effects track
    Effects,
    /// Ambient/background track
    Ambient,
    /// General/mixed audio
    General,
}

/// Audio format enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioFormat {
    Mp3,
    Wav,
    Flac,
    Aac,
    Ogg,
    M4a,
    Unknown,
}

impl AudioFormat {
    /// Detect format from file extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "mp3" => AudioFormat::Mp3,
            "wav" => AudioFormat::Wav,
            "flac" => AudioFormat::Flac,
            "aac" => AudioFormat::Aac,
            "ogg" => AudioFormat::Ogg,
            "m4a" => AudioFormat::M4a,
            _ => AudioFormat::Unknown,
        }
    }
}

/// Audio file parser
#[derive(Debug)]
pub struct AudioParser;

impl AudioParser {
    /// Create a new audio parser
    pub fn new() -> Self {
        AudioParser
    }

    /// Parse audio file from bytes
    #[instrument(skip(data), fields(size = data.len(), filename = %filename))]
    pub async fn parse(&self, data: &[u8], filename: &str) -> Result<AudioInfo> {
        info!("Parsing audio file: {}", filename);

        let format = Self::detect_format(filename);
        debug!("Detected audio format: {:?}", format);

        // Create media source
        let mss = MediaSourceStream::new(Box::new(Cursor::new(data.to_vec())), Default::default());

        // Create format hint
        let mut hint = Hint::new();
        if let Some(ext) = filename.split('.').last() {
            hint.with_extension(ext);
        }

        // Probe the media source
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|e| MediaError::AudioError(format!("Failed to probe audio: {}", e)))?;

        let format_reader = probed.format;

        // Get the default track
        let track = format_reader
            .default_track()
            .ok_or_else(|| MediaError::AudioError("No audio track found".to_string()))?;

        let codec_params = &track.codec_params;

        let sample_rate = codec_params
            .sample_rate
            .ok_or_else(|| MediaError::AudioError("No sample rate found".to_string()))?;

        let channels = codec_params
            .channels
            .map(|c| c.count() as u16)
            .unwrap_or(2);

        let bit_depth = codec_params.bits_per_sample.map(|b| b as u16);

        let codec = format!("{:?}", codec_params.codec);

        // Calculate bitrate from available parameters
        // Bitrate (bits/sec) = sample_rate * bits_per_sample * channels
        let bitrate = match (codec_params.bits_per_sample, codec_params.channels) {
            (Some(bits), Some(channels)) => {
                Some(sample_rate * bits * channels.count() as u32)
            }
            _ => None,
        };

        // Calculate duration
        let duration_seconds = if let Some(n_frames) = codec_params.n_frames {
            n_frames as f64 / sample_rate as f64
        } else {
            0.0
        };

        // Create single track (multi-track analysis would require more sophisticated parsing)
        let tracks = vec![AudioTrack {
            id: track.id,
            name: None,
            track_type: TrackType::General,
            duration_seconds,
            sample_rate,
            channels,
            start_time: 0.0,
        }];

        debug!(
            "Parsed audio: duration={:.2}s, sample_rate={}Hz, channels={}",
            duration_seconds, sample_rate, channels
        );

        Ok(AudioInfo {
            duration_seconds,
            sample_rate,
            channels,
            bit_depth,
            codec,
            bitrate,
            tracks,
            format,
        })
    }

    /// Detect audio format from filename
    fn detect_format(filename: &str) -> AudioFormat {
        filename
            .split('.')
            .last()
            .map(AudioFormat::from_extension)
            .unwrap_or(AudioFormat::Unknown)
    }

    /// Check if two audio files can be auto-merged
    pub fn can_auto_merge(base: &AudioInfo, ours: &AudioInfo, theirs: &AudioInfo) -> MergeDecision {
        // Check if track count changed
        if ours.tracks.len() != base.tracks.len() || theirs.tracks.len() != base.tracks.len() {
            warn!("Track count changed between versions");
        }

        // Find modified tracks in each branch
        let ours_modified = Self::find_modified_tracks(&base.tracks, &ours.tracks);
        let theirs_modified = Self::find_modified_tracks(&base.tracks, &theirs.tracks);

        // Check for conflicts
        let conflicts = Self::find_track_conflicts(&ours_modified, &theirs_modified);

        if conflicts.is_empty() {
            info!("No audio track conflicts detected - can auto-merge");
            MergeDecision::AutoMerge
        } else {
            warn!("Found {} audio track conflicts", conflicts.len());
            MergeDecision::ManualReview(conflicts)
        }
    }

    /// Find tracks that were modified compared to base
    fn find_modified_tracks<'a>(
        base: &'a [AudioTrack],
        modified: &'a [AudioTrack],
    ) -> Vec<&'a AudioTrack> {
        let mut result = Vec::new();

        for mod_track in modified {
            if let Some(base_track) = base.iter().find(|b| b.id == mod_track.id) {
                if Self::track_differs(base_track, mod_track) {
                    result.push(mod_track);
                }
            } else {
                // New track added
                result.push(mod_track);
            }
        }

        result
    }

    /// Check if two tracks differ
    fn track_differs(a: &AudioTrack, b: &AudioTrack) -> bool {
        (a.duration_seconds - b.duration_seconds).abs() > 0.01
            || a.sample_rate != b.sample_rate
            || a.channels != b.channels
            || (a.start_time - b.start_time).abs() > 0.01
    }

    /// Find conflicts between two sets of tracks
    fn find_track_conflicts(ours: &[&AudioTrack], theirs: &[&AudioTrack]) -> Vec<String> {
        let mut conflicts = Vec::new();

        for our_track in ours {
            for their_track in theirs {
                // Same track ID modified in both branches
                if our_track.id == their_track.id {
                    conflicts.push(format!("Track {} modified in both branches", our_track.id));
                    continue;
                }

                // Different track types can usually be auto-merged
                if our_track.track_type != their_track.track_type {
                    debug!(
                        "Different track types {:?} and {:?} - likely can merge",
                        our_track.track_type, their_track.track_type
                    );
                    continue;
                }

                // Check for temporal overlap of same track type
                let our_end = our_track.start_time + our_track.duration_seconds;
                let their_end = their_track.start_time + their_track.duration_seconds;

                if our_track.start_time < their_end && our_end > their_track.start_time {
                    conflicts.push(format!(
                        "Tracks {:?} overlap temporally at {:.2}s",
                        our_track.track_type, our_track.start_time
                    ));
                }
            }
        }

        conflicts
    }

    /// Perform actual merge of audio tracks (metadata-level merge)
    ///
    /// This merges non-conflicting audio track changes by combining:
    /// - Tracks added in 'ours' branch
    /// - Tracks added in 'theirs' branch
    /// - Tracks that don't conflict
    ///
    /// Returns merged AudioInfo structure (not actual audio binary - that requires mixing)
    pub fn merge_tracks(
        _base: &AudioInfo,
        ours: &AudioInfo,
        theirs: &AudioInfo,
    ) -> Result<AudioInfo> {
        info!("Executing audio track merge");

        // Start with ours as the base merged audio
        let mut merged_audio = AudioInfo {
            duration_seconds: ours.duration_seconds.max(theirs.duration_seconds),
            sample_rate: ours.sample_rate,
            channels: ours.channels.max(theirs.channels),
            bit_depth: ours.bit_depth,
            codec: ours.codec.clone(),
            bitrate: ours.bitrate,
            tracks: Vec::new(),
            format: ours.format,
        };

        // Merge tracks - combine all unique tracks by ID
        let mut merged_tracks = Vec::new();
        let mut track_ids = std::collections::HashSet::new();

        // Add all tracks from 'ours'
        for track in &ours.tracks {
            if track_ids.insert(track.id) {
                merged_tracks.push(track.clone());
            }
        }

        // Add new tracks from 'theirs' not in 'ours'
        for track in &theirs.tracks {
            if track_ids.insert(track.id) {
                debug!("Adding new track from 'theirs': {} ({:?})", track.id, track.track_type);
                merged_tracks.push(track.clone());
            }
        }

        merged_audio.tracks = merged_tracks;

        info!("Audio track merge complete: {} tracks", merged_audio.tracks.len());

        Ok(merged_audio)
    }
}

impl Default for AudioParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Audio merge decision
#[derive(Debug, Clone)]
pub enum MergeDecision {
    /// No conflicts, can auto-merge (e.g., different tracks like music + vocals)
    AutoMerge,
    /// Conflicts detected (same track modified in both branches)
    ManualReview(Vec<String>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_format_detection() {
        assert_eq!(AudioFormat::from_extension("mp3"), AudioFormat::Mp3);
        assert_eq!(AudioFormat::from_extension("wav"), AudioFormat::Wav);
        assert_eq!(AudioFormat::from_extension("flac"), AudioFormat::Flac);
        assert_eq!(AudioFormat::from_extension("unknown"), AudioFormat::Unknown);
    }

    #[test]
    fn test_track_type_serialization() {
        let track_type = TrackType::Vocals;
        let json = serde_json::to_string(&track_type).unwrap();
        assert!(json.contains("Vocals"));
    }

    #[test]
    fn test_track_differences() {
        let track1 = AudioTrack {
            id: 1,
            name: None,
            track_type: TrackType::Music,
            duration_seconds: 10.0,
            sample_rate: 44100,
            channels: 2,
            start_time: 0.0,
        };

        let track2 = AudioTrack {
            id: 1,
            name: None,
            track_type: TrackType::Music,
            duration_seconds: 10.5,  // Different duration
            sample_rate: 44100,
            channels: 2,
            start_time: 0.0,
        };

        assert!(AudioParser::track_differs(&track1, &track2));
    }

    #[test]
    fn test_different_track_types_no_conflict() {
        let music_track = AudioTrack {
            id: 1,
            name: Some("Music".to_string()),
            track_type: TrackType::Music,
            duration_seconds: 10.0,
            sample_rate: 44100,
            channels: 2,
            start_time: 0.0,
        };

        let vocal_track = AudioTrack {
            id: 2,
            name: Some("Vocals".to_string()),
            track_type: TrackType::Vocals,
            duration_seconds: 10.0,
            sample_rate: 44100,
            channels: 2,
            start_time: 0.0,
        };

        // Different track types should not conflict
        let conflicts = AudioParser::find_track_conflicts(&[&music_track], &[&vocal_track]);
        assert!(conflicts.is_empty());
    }
}

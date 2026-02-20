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

//! Video timeline parsing and analysis
//!
//! This module provides MP4 video file parsing and timeline analysis
//! to enable intelligent merging of non-overlapping timeline edits.
//!
//! # Features
//!
//! - MP4 container parsing
//! - Track identification (video, audio, subtitles)
//! - Timeline segment analysis
//! - Duration and timing information
//! - Conflict detection for timeline overlaps
//!
//! # Example
//!
//! ```rust,no_run
//! use mediagit_media::video::VideoParser;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let video_data = std::fs::read("video.mp4")?;
//! let parser = VideoParser::new();
//! let info = parser.parse(&video_data).await?;
//!
//! println!("Duration: {} seconds", info.duration_seconds);
//! println!("Tracks: {}", info.tracks.len());
//! # Ok(())
//! # }
//! ```

use crate::error::{MediaError, Result};
use mp4parse::{read_mp4, MediaContext, TrackType};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use tracing::{debug, info, instrument, warn};

/// Complete video file information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoInfo {
    /// Video duration in seconds
    pub duration_seconds: f64,

    /// All tracks in the video
    pub tracks: Vec<TrackInfo>,

    /// Video codec (if video track exists)
    pub video_codec: Option<String>,

    /// Audio codec (if audio track exists)
    pub audio_codec: Option<String>,

    /// Container format brand
    pub brand: String,

    /// Timeline segments for merge analysis
    pub segments: Vec<TimelineSegment>,
}

/// Information about a single track
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    /// Track ID
    pub id: u32,

    /// Track type (video, audio, subtitle, etc.)
    pub track_type: String,

    /// Track duration in seconds
    pub duration_seconds: f64,

    /// Codec information
    pub codec: String,

    /// Timescale (ticks per second)
    pub timescale: u32,

    /// For video: width
    pub width: Option<u32>,

    /// For video: height
    pub height: Option<u32>,

    /// For audio: sample rate
    pub sample_rate: Option<u32>,

    /// For audio: number of channels
    pub channels: Option<u16>,
}

/// Timeline segment for merge analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineSegment {
    /// Track ID this segment belongs to
    pub track_id: u32,

    /// Segment start time in seconds
    pub start_time: f64,

    /// Segment end time in seconds
    pub end_time: f64,

    /// Segment duration
    pub duration: f64,

    /// Segment type (e.g., "video", "audio", "keyframe")
    pub segment_type: String,
}

impl TimelineSegment {
    /// Check if this segment overlaps with another
    pub fn overlaps(&self, other: &TimelineSegment) -> bool {
        self.track_id == other.track_id
            && self.start_time < other.end_time
            && self.end_time > other.start_time
    }

    /// Calculate overlap duration with another segment
    pub fn overlap_duration(&self, other: &TimelineSegment) -> f64 {
        if !self.overlaps(other) {
            return 0.0;
        }

        let overlap_start = self.start_time.max(other.start_time);
        let overlap_end = self.end_time.min(other.end_time);

        (overlap_end - overlap_start).max(0.0)
    }
}

/// Video file parser
#[derive(Debug)]
pub struct VideoParser;

impl VideoParser {
    /// Create a new video parser
    pub fn new() -> Self {
        VideoParser
    }

    /// Parse MP4 video file from bytes
    #[instrument(skip(data), fields(size = data.len()))]
    pub async fn parse(&self, data: &[u8]) -> Result<VideoInfo> {
        info!("Parsing MP4 video file");

        let mut cursor = Cursor::new(data);
        let context = read_mp4(&mut cursor)
            .map_err(|e| MediaError::VideoError(format!("Failed to parse MP4: {:?}", e)))?;

        let tracks = self.extract_tracks(&context)?;
        let segments = self.extract_segments(&tracks);

        let duration_seconds = tracks
            .first()
            .map(|t| t.duration_seconds)
            .unwrap_or(0.0);

        let video_codec = tracks
            .iter()
            .find(|t| t.track_type == "video")
            .map(|t| t.codec.clone());

        let audio_codec = tracks
            .iter()
            .find(|t| t.track_type == "audio")
            .map(|t| t.codec.clone());

        // Note: ftyp field removed from MediaContext in mp4parse 0.17
        // Using generic brand identifier instead
        let brand = "mp4".to_string();

        debug!(
            "Parsed video: duration={:.2}s, tracks={}, brand={}",
            duration_seconds,
            tracks.len(),
            brand
        );

        Ok(VideoInfo {
            duration_seconds,
            tracks,
            video_codec,
            audio_codec,
            brand,
            segments,
        })
    }

    /// Extract track information from MP4 context
    fn extract_tracks(&self, context: &MediaContext) -> Result<Vec<TrackInfo>> {
        let mut tracks = Vec::new();

        for track in &context.tracks {
            let track_id = track.track_id.unwrap_or(0);
            let track_type = Self::track_type_string(&track.track_type);

            // Extract timescale value from TrackTimeScale wrapper
            let timescale = track.timescale
                .map(|ts| ts.0 as u32)
                .unwrap_or(1000);

            // Extract duration value from TrackScaledTime wrapper
            let duration = track.duration
                .map(|d| d.0)
                .unwrap_or(0);

            let duration_seconds = duration as f64 / timescale as f64;

            // Extract codec from SampleDescriptionBox
            let codec = track
                .stsd
                .as_ref()
                .and_then(|stsd| stsd.descriptions.first())
                .map(|entry| match entry {
                    mp4parse::SampleEntry::Audio(audio) => format!("{:?}", audio.codec_type),
                    mp4parse::SampleEntry::Video(video) => format!("{:?}", video.codec_type),
                    mp4parse::SampleEntry::Unknown => "unknown".to_string(),
                })
                .unwrap_or_else(|| "unknown".to_string());

            // Extract width/height from track header for video tracks
            let (width, height) = if track.track_type == TrackType::Video {
                track.tkhd.as_ref().map(|tkhd| (Some(tkhd.width), Some(tkhd.height))).unwrap_or((None, None))
            } else {
                (None, None)
            };

            // Extract sample rate and channels from audio sample entry
            let (sample_rate, channels) = if track.track_type == TrackType::Audio {
                track
                    .stsd
                    .as_ref()
                    .and_then(|stsd| stsd.descriptions.first())
                    .and_then(|entry| match entry {
                        mp4parse::SampleEntry::Audio(audio) => {
                            Some((Some(audio.samplerate as u32), Some(audio.channelcount as u16)))
                        }
                        _ => None,
                    })
                    .unwrap_or((None, None))
            } else {
                (None, None)
            };

            tracks.push(TrackInfo {
                id: track_id,
                track_type,
                duration_seconds,
                codec,
                timescale,
                width,
                height,
                sample_rate,
                channels,
            });
        }

        info!("Extracted {} tracks from video", tracks.len());
        Ok(tracks)
    }

    /// Convert track type to string
    fn track_type_string(track_type: &TrackType) -> String {
        match track_type {
            TrackType::Video => "video".to_string(),
            TrackType::Audio => "audio".to_string(),
            TrackType::Picture => "picture".to_string(),
            TrackType::AuxiliaryVideo => "auxiliary_video".to_string(),
            TrackType::Metadata => "metadata".to_string(),
            TrackType::Unknown => "unknown".to_string(),
        }
    }

    /// Extract timeline segments from tracks
    fn extract_segments(&self, tracks: &[TrackInfo]) -> Vec<TimelineSegment> {
        let mut segments = Vec::new();

        // Create simplified segments (one per track for now)
        // A more sophisticated implementation would parse actual edit lists and samples
        for track in tracks {
            segments.push(TimelineSegment {
                track_id: track.id,
                start_time: 0.0,
                end_time: track.duration_seconds,
                duration: track.duration_seconds,
                segment_type: track.track_type.clone(),
            });
        }

        segments
    }

    /// Check if two videos can be auto-merged based on timeline
    pub fn can_auto_merge(base: &VideoInfo, ours: &VideoInfo, theirs: &VideoInfo) -> MergeDecision {
        // Check for track count changes
        if ours.tracks.len() != base.tracks.len() || theirs.tracks.len() != base.tracks.len() {
            warn!("Track count changed between versions");
        }

        // Find modified segments in each branch
        let ours_modified = Self::find_modified_segments(&base.segments, &ours.segments);
        let theirs_modified = Self::find_modified_segments(&base.segments, &theirs.segments);

        // Check for timeline conflicts
        let conflicts = Self::find_timeline_conflicts(&ours_modified, &theirs_modified);

        if conflicts.is_empty() {
            info!("No timeline conflicts detected - can auto-merge");
            MergeDecision::AutoMerge
        } else {
            warn!("Found {} timeline conflicts", conflicts.len());
            MergeDecision::ManualReview(conflicts)
        }
    }

    /// Find segments that were modified compared to base
    fn find_modified_segments<'a>(
        base: &'a [TimelineSegment],
        modified: &'a [TimelineSegment],
    ) -> Vec<&'a TimelineSegment> {
        let mut result = Vec::new();

        for mod_seg in modified {
            if let Some(base_seg) = base.iter().find(|b| b.track_id == mod_seg.track_id) {
                if Self::segment_differs(base_seg, mod_seg) {
                    result.push(mod_seg);
                }
            } else {
                // New segment added
                result.push(mod_seg);
            }
        }

        result
    }

    /// Check if two segments differ
    fn segment_differs(a: &TimelineSegment, b: &TimelineSegment) -> bool {
        (a.start_time - b.start_time).abs() > 0.01
            || (a.end_time - b.end_time).abs() > 0.01
            || (a.duration - b.duration).abs() > 0.01
    }

    /// Find timeline conflicts between two sets of segments
    fn find_timeline_conflicts(
        ours: &[&TimelineSegment],
        theirs: &[&TimelineSegment],
    ) -> Vec<String> {
        let mut conflicts = Vec::new();

        for our_seg in ours {
            for their_seg in theirs {
                if our_seg.overlaps(their_seg) {
                    let overlap = our_seg.overlap_duration(their_seg);
                    conflicts.push(format!(
                        "Track {} has overlapping edits: {:.2}s overlap at {:.2}s",
                        our_seg.track_id, overlap, our_seg.start_time
                    ));
                }
            }
        }

        conflicts
    }

    /// Perform actual merge of video timelines (metadata-level merge)
    ///
    /// This merges non-conflicting timeline changes by combining:
    /// - Tracks added in 'ours' branch
    /// - Tracks added in 'theirs' branch
    /// - Timeline segments that don't overlap
    ///
    /// Returns merged VideoInfo structure (not actual video binary - that requires re-encoding)
    pub fn merge_timelines(
        _base: &VideoInfo,
        ours: &VideoInfo,
        theirs: &VideoInfo,
    ) -> Result<VideoInfo> {
        info!("Executing video timeline merge");

        // Start with ours as the base merged video
        let mut merged_video = VideoInfo {
            duration_seconds: ours.duration_seconds.max(theirs.duration_seconds),
            tracks: Vec::new(),
            video_codec: ours.video_codec.clone(),
            audio_codec: ours.audio_codec.clone(),
            brand: ours.brand.clone(),
            segments: Vec::new(),
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
                debug!("Adding new track from 'theirs': {}", track.id);
                merged_tracks.push(track.clone());
            }
        }

        merged_video.tracks = merged_tracks;

        // Merge segments - combine non-overlapping segments
        let mut merged_segments = Vec::new();

        // Add all segments from 'ours'
        for segment in &ours.segments {
            merged_segments.push(segment.clone());
        }

        // Add segments from 'theirs' that don't overlap with 'ours'
        for their_seg in &theirs.segments {
            let overlaps = merged_segments.iter().any(|our_seg| their_seg.overlaps(our_seg));
            if !overlaps {
                debug!("Adding non-overlapping segment from 'theirs': track {}", their_seg.track_id);
                merged_segments.push(their_seg.clone());
            }
        }

        merged_video.segments = merged_segments;

        info!("Video timeline merge complete: {} tracks, {} segments",
              merged_video.tracks.len(), merged_video.segments.len());

        Ok(merged_video)
    }
}

impl Default for VideoParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Video merge decision
#[derive(Debug, Clone)]
pub enum MergeDecision {
    /// No timeline conflicts, can auto-merge
    AutoMerge,
    /// Timeline conflicts detected, needs manual review
    ManualReview(Vec<String>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeline_segment_overlap() {
        let seg1 = TimelineSegment {
            track_id: 1,
            start_time: 0.0,
            end_time: 10.0,
            duration: 10.0,
            segment_type: "video".to_string(),
        };

        let seg2 = TimelineSegment {
            track_id: 1,
            start_time: 5.0,
            end_time: 15.0,
            duration: 10.0,
            segment_type: "video".to_string(),
        };

        assert!(seg1.overlaps(&seg2));
        assert_eq!(seg1.overlap_duration(&seg2), 5.0);
    }

    #[test]
    fn test_timeline_segment_no_overlap() {
        let seg1 = TimelineSegment {
            track_id: 1,
            start_time: 0.0,
            end_time: 10.0,
            duration: 10.0,
            segment_type: "video".to_string(),
        };

        let seg2 = TimelineSegment {
            track_id: 1,
            start_time: 20.0,
            end_time: 30.0,
            duration: 10.0,
            segment_type: "video".to_string(),
        };

        assert!(!seg1.overlaps(&seg2));
        assert_eq!(seg1.overlap_duration(&seg2), 0.0);
    }

    #[test]
    fn test_different_track_no_conflict() {
        let seg1 = TimelineSegment {
            track_id: 1,
            start_time: 0.0,
            end_time: 10.0,
            duration: 10.0,
            segment_type: "video".to_string(),
        };

        let seg2 = TimelineSegment {
            track_id: 2,
            start_time: 0.0,
            end_time: 10.0,
            duration: 10.0,
            segment_type: "audio".to_string(),
        };

        // Different tracks don't conflict
        assert!(!seg1.overlaps(&seg2));
    }
}

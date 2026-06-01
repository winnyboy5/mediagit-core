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

use super::*;

impl ObjectDatabase {
    /// Write an object with delta compression if similar object found
    ///
    /// Attempts to find a similar object in recent history and stores only the
    /// delta (difference) if similarity exceeds threshold.
    ///
    /// # Arguments
    ///
    /// * `obj_type` - The type of object being written
    /// * `data` - The object content
    /// * `filename` - Optional filename for metadata
    ///
    /// # Returns
    ///
    /// The OID of the stored object
    ///
    /// # Delta Compression Logic
    ///
    /// 1. Generate samples from the new object
    /// 2. Search recent objects for similarity (> 30%)
    /// 3. If similar object found, create delta
    /// 4. Use delta only if smaller than 80% of original
    /// 5. Fall back to standard write otherwise
    pub async fn write_with_delta(
        &self,
        obj_type: ObjectType,
        data: &[u8],
        filename: &str,
    ) -> anyhow::Result<Oid> {
        if !self.delta_enabled {
            // Delta not enabled, fall back to standard write
            return self.write_with_path(obj_type, data, filename).await;
        }

        let oid = Oid::hash(data);

        debug!(
            oid = %oid,
            size = data.len(),
            filename,
            "Attempting delta compression"
        );

        // Create metadata and generate samples for similarity detection
        let mut metadata = crate::similarity::ObjectMetadata::new(
            oid,
            data.len(),
            obj_type,
            if filename.is_empty() {
                None
            } else {
                Some(filename.to_string())
            },
        );
        metadata.generate_samples(data);

        // Find similar object for delta base
        let detector = self.similarity_detector.read().await;
        let threshold = crate::similarity::get_similarity_threshold(if filename.is_empty() {
            None
        } else {
            Some(filename)
        });
        let size_ratio = crate::similarity::get_size_ratio_threshold(if filename.is_empty() {
            None
        } else {
            Some(filename)
        });
        let similar = detector.find_similar_with_size_ratio(&metadata, threshold, size_ratio);
        drop(detector);

        if let Some((base_oid, score)) = similar {
            // CRITICAL: Prevent self-referencing delta (OID == base OID)
            if oid == base_oid {
                warn!(
                    oid = %oid,
                    "Attempted to create self-referencing delta, storing as full object"
                );
            } else {
                info!(
                    oid = %oid,
                    base_oid = %base_oid,
                    similarity = score.score,
                    "Found similar object, attempting delta compression"
                );

                // Read base object
                match self.read(&base_oid).await {
                    Ok(base_data) => {
                        // Check delta chain depth - prevent unbounded chains
                        let base_depth = self.get_delta_depth(&base_oid).await.unwrap_or(0);

                        // Also check if base's chain already contains this OID (would create cycle)
                        let would_create_cycle = self
                            .delta_chain_contains(&base_oid, &oid)
                            .await
                            .unwrap_or(false);

                        if would_create_cycle {
                            warn!(
                                oid = %oid,
                                base_oid = %base_oid,
                                "Base's delta chain already contains this OID, storing as full object to prevent cycle"
                            );
                            // Fall through to standard write
                        } else if base_depth >= MAX_DELTA_DEPTH {
                            info!(
                                oid = %oid,
                                base_oid = %base_oid,
                                depth = base_depth,
                                max_depth = MAX_DELTA_DEPTH,
                                "Delta chain limit reached, storing as full object"
                            );
                            // Fall through to standard write
                        } else {
                            // Create delta
                            let delta = DeltaEncoder::encode(&base_data, data);
                            let delta_data = delta.to_bytes();

                            // Only use delta if it's smaller than 80% of original
                            let delta_ratio = delta_data.len() as f64 / data.len() as f64;

                            if delta_ratio < 0.80 {
                                info!(
                                    oid = %oid,
                                    original_size = data.len(),
                                    delta_size = delta_data.len(),
                                    ratio = delta_ratio,
                                    "Delta compression beneficial, storing delta"
                                );

                                // Store delta
                                let delta_key = format!("deltas/{}", oid.to_hex());
                                let compressed_delta =
                                    if let Some(smart_comp) = &self.smart_compressor {
                                        smart_comp.compress_typed(
                                            &delta_data,
                                            CompressionObjectType::Unknown,
                                        )?
                                    } else {
                                        self.compressor.compress(&delta_data)?
                                    };

                                self.storage.put(&delta_key, &compressed_delta).await?;

                                // Store delta metadata (base OID reference + chain depth)
                                let new_depth = base_depth + 1;
                                let delta_meta =
                                    format!("base:{}:depth:{}", base_oid.to_hex(), new_depth);
                                let meta_key = format!("deltas/{}.meta", oid.to_hex());
                                self.storage.put(&meta_key, delta_meta.as_bytes()).await?;

                                debug!(
                                    oid = %oid,
                                    base_oid = %base_oid,
                                    depth = new_depth,
                                    "Stored delta with chain depth"
                                );

                                // Update metrics
                                let mut metrics = self.metrics.write().await;
                                metrics.record_write(data.len() as u64, true);

                                // Cache original data (skip large objects)
                                if data.len() <= MAX_CACHEABLE_OBJECT_SIZE {
                                    self.cache.insert(oid, Arc::new(data.to_vec())).await;
                                }

                                // Add to similarity detector for future matching
                                let mut detector = self.similarity_detector.write().await;
                                detector.add_object(metadata);

                                return Ok(oid);
                            } else {
                                debug!(
                                    oid = %oid,
                                    delta_ratio,
                                    "Delta not beneficial, using standard storage"
                                );
                            }
                        } // Close depth check else branch
                    }
                    Err(e) => {
                        warn!(
                            oid = %oid,
                            base_oid = %base_oid,
                            error = %e,
                            "Failed to read base object, using standard storage"
                        );
                    }
                }
            } // Close else block for oid != base_oid check
        }

        // No similar object found or delta not beneficial
        // Add metadata to detector for future comparisons
        let mut detector = self.similarity_detector.write().await;
        detector.add_object(metadata);
        drop(detector);

        // Fall back to standard write
        self.write_with_path(obj_type, data, filename).await
    }

    /// Get the delta chain depth for an object
    ///
    /// Returns 0 if object is not a delta (full object).
    /// Returns the chain depth if object is stored as a delta.
    ///
    /// # Arguments
    ///
    /// * `oid` - Object identifier to check
    ///
    /// # Returns
    ///
    /// The delta chain depth (0 = full object, 1+ = delta depth)
    async fn get_delta_depth(&self, oid: &Oid) -> anyhow::Result<u8> {
        let meta_key = format!("deltas/{}.meta", oid.to_hex());

        // Check if delta metadata exists
        if !self.storage.exists(&meta_key).await? {
            return Ok(0); // Not a delta, depth is 0
        }

        // Read and parse metadata
        let meta_data = self.storage.get(&meta_key).await?;
        let meta_str = String::from_utf8(meta_data).map_err(|e| {
            anyhow::anyhow!("Invalid delta metadata encoding in get_delta_depth: {}", e)
        })?;

        // Parse format: "base:{oid}:depth:{n}" or legacy "base:{oid}"
        if let Some(depth_part) = meta_str.split(":depth:").nth(1) {
            // Trim to handle any trailing whitespace/newlines
            let trimmed = depth_part.trim();
            match trimmed.parse::<u8>() {
                Ok(depth) => Ok(depth),
                Err(e) => {
                    warn!(meta_str = %meta_str, error = %e, "Failed to parse delta depth, defaulting to 1");
                    Ok(1)
                }
            }
        } else {
            // Legacy format without depth, assume depth 1
            Ok(1)
        }
    }

    /// Check if a target OID exists in the delta chain starting from a given OID
    ///
    /// This is used to prevent creating circular delta references.
    /// Walks the delta chain from `start_oid` and returns true if `target_oid` is found.
    ///
    /// # Arguments
    ///
    /// * `start_oid` - Starting point of the delta chain to check
    /// * `target_oid` - OID to search for in the chain
    ///
    /// # Returns
    ///
    /// True if target_oid is found in the chain, false otherwise
    async fn delta_chain_contains(
        &self,
        start_oid: &Oid,
        target_oid: &Oid,
    ) -> anyhow::Result<bool> {
        let mut current_oid = *start_oid;
        let mut visited = std::collections::HashSet::new();

        // Walk the chain with depth limit to prevent infinite loops
        for _ in 0..=MAX_DELTA_DEPTH {
            // Check if current matches target
            if current_oid == *target_oid {
                return Ok(true);
            }

            // Check for cycles in our walk
            if !visited.insert(current_oid) {
                // Already visited this OID, we're in a cycle (shouldn't happen but be safe)
                return Ok(false);
            }

            // Try to get the base OID of current
            let meta_key = format!("deltas/{}.meta", current_oid.to_hex());
            if !self.storage.exists(&meta_key).await? {
                // Not a delta, end of chain
                return Ok(false);
            }

            // Parse base OID from metadata
            let meta_data = self.storage.get(&meta_key).await?;
            let meta_str = String::from_utf8(meta_data)?;
            let after_prefix = meta_str
                .strip_prefix("base:")
                .ok_or_else(|| anyhow::anyhow!("Invalid delta metadata format"))?
                .trim();

            // Handle both formats: "base:{oid}:depth:{n}" and legacy "base:{oid}"
            let base_oid_hex = if let Some(idx) = after_prefix.find(":depth:") {
                &after_prefix[..idx]
            } else {
                after_prefix
            };

            current_oid = Oid::from_hex(base_oid_hex)?;
        }

        // Exceeded depth limit without finding target
        Ok(false)
    }

    /// Check whether `target` appears in the on-disk chunk-delta chain starting
    /// at `start`. Used to prevent writing a chunk-delta whose base chain
    /// already leads back to the new chunk (which would produce the 2-cycle
    /// observed in parallel add workflows).
    ///
    /// Bounded by `MAX_DELTA_DEPTH`; treats malformed meta / missing base as
    /// "not in chain" (terminal) — we are only interested in *our* OID
    /// appearing on the path.
    pub async fn chunk_delta_chain_contains(
        &self,
        start: &Oid,
        target: &Oid,
    ) -> anyhow::Result<bool> {
        Ok(chunk_delta_chain_contains_impl(&*self.storage, *start, *target).await)
    }

    /// Reconstruct a delta-encoded object
    ///
    /// Reads the delta metadata to find the base object, then applies
    /// the delta to reconstruct the original object.
    ///
    /// # Arguments
    ///
    /// * `oid` - Object identifier of the delta-encoded object
    ///
    /// # Returns
    ///
    /// The reconstructed object content
    pub(super) async fn read_delta(&self, oid: &Oid) -> anyhow::Result<Vec<u8>> {
        debug!(oid = %oid, "Reconstructing delta-encoded object");

        // Read delta metadata to get base OID
        let meta_key = format!("deltas/{}.meta", oid.to_hex());
        let meta_data = self
            .storage
            .get(&meta_key)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read delta metadata for {}: {}", oid, e))?;

        // Parse base OID from metadata (format: "base:{hex_oid}:depth:{n}" or legacy "base:{hex_oid}")
        let meta_str = String::from_utf8(meta_data)
            .map_err(|e| anyhow::anyhow!("Invalid delta metadata encoding: {}", e))?;

        let after_prefix = meta_str
            .strip_prefix("base:")
            .ok_or_else(|| anyhow::anyhow!("Invalid delta metadata format: {}", meta_str))?
            .trim();

        // Handle both formats: "base:{oid}:depth:{n}" and legacy "base:{oid}"
        let base_oid_hex = if let Some(idx) = after_prefix.find(":depth:") {
            &after_prefix[..idx]
        } else {
            after_prefix
        };

        let base_oid = Oid::from_hex(base_oid_hex)
            .map_err(|e| anyhow::anyhow!("Invalid base OID in delta metadata: {}", e))?;

        debug!(
            oid = %oid,
            base_oid = %base_oid,
            "Found delta base object"
        );

        // Read base object (this may recursively read another delta, with depth limit)
        // To prevent infinite recursion, we track depth via a simple counter
        let base_data = self.read_delta_with_depth(&base_oid, 1).await?;

        // Read delta data
        let delta_key = format!("deltas/{}", oid.to_hex());
        let compressed_delta = self
            .storage
            .get(&delta_key)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read delta data for {}: {}", oid, e))?;

        // Decompress delta
        let delta_bytes = if let Some(smart_comp) = &self.smart_compressor {
            decompress_typed_blocking(smart_comp.clone(), compressed_delta)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to decompress delta: {}", e))?
        } else {
            decompress_blocking(self.compressor.clone(), compressed_delta)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to decompress delta: {}", e))?
        };

        // Parse and apply delta
        let delta = Delta::from_bytes(&delta_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to parse delta: {}", e))?;

        let reconstructed = DeltaDecoder::apply(&base_data, &delta)
            .map_err(|e| anyhow::anyhow!("Failed to apply delta: {}", e))?;

        // Verify integrity
        let computed_oid = Oid::hash(&reconstructed);
        if computed_oid != *oid {
            anyhow::bail!(
                "Delta reconstruction failed: expected OID {}, computed {}",
                oid,
                computed_oid
            );
        }

        info!(
            oid = %oid,
            base_oid = %base_oid,
            base_size = base_data.len(),
            delta_size = delta_bytes.len(),
            result_size = reconstructed.len(),
            "Successfully reconstructed delta-encoded object"
        );

        // Cache reconstructed data (skip large objects)
        if reconstructed.len() <= MAX_CACHEABLE_OBJECT_SIZE {
            self.cache
                .insert(*oid, Arc::new(reconstructed.clone()))
                .await;
        }

        Ok(reconstructed)
    }

    /// Read delta with depth tracking and circular reference detection
    ///
    /// Maximum delta chain depth is 10 levels (consistent with MAX_DELTA_DEPTH).
    /// Uses Box::pin to handle async recursion.
    /// Tracks visited OIDs to detect actual circular references.
    fn read_delta_with_depth(
        &self,
        oid: &Oid,
        depth: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<u8>>> + Send + '_>>
    {
        let oid = *oid;
        // Create a new visited set for the initial call
        let visited = std::collections::HashSet::new();
        self.read_delta_with_depth_internal(oid, depth, visited)
    }

    /// Internal delta reading with visited set for circular reference detection
    fn read_delta_with_depth_internal(
        &self,
        oid: Oid,
        depth: usize,
        mut visited: std::collections::HashSet<Oid>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<u8>>> + Send + '_>>
    {
        Box::pin(async move {
            // Use the same limit as the write side (MAX_DELTA_DEPTH = 10)
            const MAX_DELTA_CHAIN_DEPTH: usize = MAX_DELTA_DEPTH as usize;

            // Check for circular reference FIRST (before depth check)
            if !visited.insert(oid) {
                anyhow::bail!("Circular reference detected in delta chain at OID {}", oid);
            }

            if depth > MAX_DELTA_CHAIN_DEPTH {
                anyhow::bail!(
                    "Delta chain too deep (> {}): chain starting at {}",
                    MAX_DELTA_CHAIN_DEPTH,
                    oid
                );
            }

            // Check cache first
            if let Some(cached) = self.cache.get(&oid).await {
                return Ok((*cached).clone());
            }

            // Check if this is also a delta
            let meta_key = format!("deltas/{}.meta", oid.to_hex());
            if self.storage.exists(&meta_key).await? {
                // Read delta metadata (format: "base:{oid}:depth:{n}" or legacy "base:{oid}")
                let meta_data = self.storage.get(&meta_key).await?;
                let meta_str = String::from_utf8(meta_data)?;
                let after_prefix = meta_str
                    .strip_prefix("base:")
                    .ok_or_else(|| anyhow::anyhow!("Invalid delta metadata format"))?
                    .trim();
                // Handle both formats
                let base_oid_hex = if let Some(idx) = after_prefix.find(":depth:") {
                    &after_prefix[..idx]
                } else {
                    after_prefix
                };
                let base_oid = Oid::from_hex(base_oid_hex)?;

                // Recursively read base with incremented depth, passing the visited set
                let base_data = self
                    .read_delta_with_depth_internal(base_oid, depth + 1, visited)
                    .await?;

                // Read and apply delta
                let delta_key = format!("deltas/{}", oid.to_hex());
                let compressed_delta = self.storage.get(&delta_key).await?;
                let delta_bytes = if let Some(smart_comp) = &self.smart_compressor {
                    decompress_typed_blocking(smart_comp.clone(), compressed_delta).await?
                } else {
                    decompress_blocking(self.compressor.clone(), compressed_delta).await?
                };

                let delta = Delta::from_bytes(&delta_bytes)?;
                let reconstructed = DeltaDecoder::apply(&base_data, &delta)?;

                // Cache and return
                self.cache
                    .insert(oid, Arc::new(reconstructed.clone()))
                    .await;
                return Ok(reconstructed);
            }

            // Not a delta - try other storage methods
            // Check chunk manifest
            let manifest_key = format!("manifests/{}", oid.to_hex());
            if self.storage.exists(&manifest_key).await? {
                return self.read_chunked(&oid).await;
            }

            // Try loose object
            let key = oid.to_hex();
            if let Ok(storage_data) = self.storage.get(&key).await {
                let data = if let Some(smart_comp) = &self.smart_compressor {
                    decompress_typed_blocking(smart_comp.clone(), storage_data.clone())
                        .await
                        .unwrap_or(storage_data)
                } else {
                    let fallback = storage_data.clone();
                    decompress_blocking(self.compressor.clone(), storage_data)
                        .await
                        .unwrap_or(fallback)
                };

                if data.len() <= MAX_CACHEABLE_OBJECT_SIZE {
                    self.cache.insert(oid, Arc::new(data.clone())).await;
                }
                return Ok(data);
            }

            // Try pack files
            self.read_from_packs(&oid).await
        })
    }
}

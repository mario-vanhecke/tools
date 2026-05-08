use crate::config::EmbeddingDevice;
use crate::error::{Error, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::xlm_roberta::{Config as XlmCfg, XLMRobertaModel};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokenizers::Tokenizer;

use super::Embedder;

/// Embedder backed by Candle. Defaults to bge-m3 (an XLM-RoBERTa derivative).
pub struct CandleEmbedder {
    model_id: String,
    dimension: u32,
    device: Device,
    tokenizer: Tokenizer,
    model: Mutex<XLMRobertaModel>,
    max_len: usize,
    batch_size: usize,
}

impl CandleEmbedder {
    /// Construct an embedder. If model files aren't present in `cache_dir`,
    /// download from Hugging Face. Reports progress via `progress` if provided.
    pub fn load(
        model_id: &str,
        device_pref: EmbeddingDevice,
        cache_dir: &Path,
        batch_size: u32,
        progress: Option<&dyn Fn(&str)>,
    ) -> Result<Self> {
        let device = pick_device(device_pref)?;
        let (config_path, tokenizer_path, weights_path) =
            ensure_model_files(model_id, cache_dir, progress)?;

        let config_bytes = std::fs::read(&config_path).map_err(Error::from)?;
        let config: XlmCfg =
            serde_json::from_slice(&config_bytes).map_err(|e| Error::embedder(e.to_string()))?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| Error::embedder(format!("tokenizer: {e}")))?;

        // bge-m3 ships `pytorch_model.bin` (PyTorch pickle format); other
        // models ship `model.safetensors`. Pick whichever was downloaded.
        let is_pth = weights_path.extension().and_then(|e| e.to_str()) == Some("bin");
        let vb = if is_pth {
            VarBuilder::from_pth(&weights_path, DType::F32, &device)
                .map_err(|e| Error::embedder(e.to_string()))?
        } else {
            unsafe {
                VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)
                    .map_err(|e| Error::embedder(e.to_string()))?
            }
        };
        let model =
            XLMRobertaModel::new(&config, vb).map_err(|e| Error::embedder(e.to_string()))?;

        let dimension = config.hidden_size as u32;

        Ok(Self {
            model_id: model_id.to_string(),
            dimension,
            device,
            tokenizer,
            model: Mutex::new(model),
            max_len: 512,
            batch_size: batch_size.max(1) as usize,
        })
    }
}

impl Embedder for CandleEmbedder {
    fn dimension(&self) -> u32 {
        self.dimension
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
        for batch in texts.chunks(self.batch_size) {
            let encs = self
                .tokenizer
                .encode_batch(batch.to_vec(), true)
                .map_err(|e| Error::embedder(format!("tokenize: {e}")))?;

            // Truncate / pad to `max_len`. The HF tokenizer config typically
            // pads/truncates already; we re-clamp to be defensive.
            let max_len = encs
                .iter()
                .map(|e| e.get_ids().len())
                .max()
                .unwrap_or(0)
                .min(self.max_len);

            if max_len == 0 {
                for _ in 0..batch.len() {
                    out.push(vec![0.0; self.dimension as usize]);
                }
                continue;
            }

            let mut input_ids: Vec<u32> = Vec::with_capacity(batch.len() * max_len);
            let mut attn_mask: Vec<u32> = Vec::with_capacity(batch.len() * max_len);
            for e in &encs {
                let ids = e.get_ids();
                let am = e.get_attention_mask();
                let take = ids.len().min(max_len);
                input_ids.extend_from_slice(&ids[..take]);
                attn_mask.extend_from_slice(&am[..take]);
                for _ in take..max_len {
                    input_ids.push(0);
                    attn_mask.push(0);
                }
            }

            let bsz = batch.len();
            let ids = Tensor::from_vec(input_ids, (bsz, max_len), &self.device)
                .map_err(|e| Error::embedder(e.to_string()))?;
            let am = Tensor::from_vec(attn_mask.clone(), (bsz, max_len), &self.device)
                .map_err(|e| Error::embedder(e.to_string()))?;
            let token_type_ids = Tensor::zeros((bsz, max_len), DType::U32, &self.device)
                .map_err(|e| Error::embedder(e.to_string()))?;

            let model = self
                .model
                .lock()
                .map_err(|_| Error::embedder("model mutex poisoned"))?;
            let hidden = model
                .forward(&ids, &am, &token_type_ids, None, None, None)
                .map_err(|e| Error::embedder(e.to_string()))?;

            // Mean-pool over the attention mask. Candle requires explicit
            // broadcast operators when shapes differ.
            let am_f = am
                .to_dtype(DType::F32)
                .map_err(|e| Error::embedder(e.to_string()))?
                .unsqueeze(2)
                .map_err(|e| Error::embedder(e.to_string()))?; // (bsz, len, 1)
            let masked = hidden
                .broadcast_mul(&am_f)
                .map_err(|e| Error::embedder(e.to_string()))?;
            let summed = masked.sum(1).map_err(|e| Error::embedder(e.to_string()))?; // (bsz, 1024)
            let counts = am_f
                .sum(1)
                .map_err(|e| Error::embedder(e.to_string()))?
                .clamp(1e-9, f32::INFINITY)
                .map_err(|e| Error::embedder(e.to_string()))?; // (bsz, 1)
            let mean = summed
                .broadcast_div(&counts)
                .map_err(|e| Error::embedder(e.to_string()))?;

            // L2-normalize each row.
            let norm = mean
                .sqr()
                .map_err(|e| Error::embedder(e.to_string()))?
                .sum_keepdim(1)
                .map_err(|e| Error::embedder(e.to_string()))?
                .sqrt()
                .map_err(|e| Error::embedder(e.to_string()))?
                .clamp(1e-9, f32::INFINITY)
                .map_err(|e| Error::embedder(e.to_string()))?;
            let normed = mean
                .broadcast_div(&norm)
                .map_err(|e| Error::embedder(e.to_string()))?;

            let vecs: Vec<Vec<f32>> = normed
                .to_vec2::<f32>()
                .map_err(|e| Error::embedder(e.to_string()))?;
            out.extend(vecs);
        }
        Ok(out)
    }
}

fn pick_device(pref: EmbeddingDevice) -> Result<Device> {
    let device = match pref {
        EmbeddingDevice::Cpu => Device::Cpu,
        EmbeddingDevice::Metal => {
            Device::new_metal(0).map_err(|e| Error::embedder(e.to_string()))?
        }
        EmbeddingDevice::Cuda => Device::new_cuda(0).map_err(|e| Error::embedder(e.to_string()))?,
        EmbeddingDevice::Auto => {
            #[cfg(target_os = "macos")]
            let metal = Device::new_metal(0);
            #[cfg(not(target_os = "macos"))]
            let metal: std::result::Result<Device, candle_core::Error> =
                Err(candle_core::Error::Msg("metal: not on macos".to_string()));
            #[cfg(not(target_os = "macos"))]
            let cuda = Device::new_cuda(0);
            #[cfg(target_os = "macos")]
            let cuda: std::result::Result<Device, candle_core::Error> = Err(
                candle_core::Error::Msg("cuda: not on macos build".to_string()),
            );

            match (metal, cuda) {
                (Ok(d), _) => d,
                (_, Ok(d)) => d,
                (Err(e1), Err(e2)) => {
                    tracing::info!(
                        "embedder: GPU unavailable (metal: {}, cuda: {}); using CPU. \
                         Build with `--features metal` (macOS) or `--features cuda` (Linux) for acceleration.",
                        e1,
                        e2
                    );
                    Device::Cpu
                }
            }
        }
    };
    let label = match &device {
        Device::Cpu => "cpu",
        Device::Metal(_) => "metal",
        Device::Cuda(_) => "cuda",
    };
    tracing::info!("embedder: device = {}", label);
    Ok(device)
}

fn ensure_model_files(
    model_id: &str,
    cache_dir: &Path,
    progress: Option<&dyn Fn(&str)>,
) -> Result<(PathBuf, PathBuf, PathBuf)> {
    // Local layout: <cache_dir>/<safe-id>/{config.json,tokenizer.json,model.safetensors}
    let safe = model_id.replace('/', "__");
    let local = cache_dir.join(&safe);
    std::fs::create_dir_all(&local)?;

    let cfg = local.join("config.json");
    let tok = local.join("tokenizer.json");
    let weights_st = local.join("model.safetensors");
    let weights_pt = local.join("pytorch_model.bin");

    let weights_existing = if weights_st.exists() {
        Some(weights_st.clone())
    } else if weights_pt.exists() {
        Some(weights_pt.clone())
    } else {
        None
    };

    if let (true, true, Some(w)) = (cfg.exists(), tok.exists(), weights_existing) {
        return Ok((cfg, tok, w));
    }

    if let Some(p) = progress {
        p(&format!("downloading {model_id} ..."));
    }

    for (name, dest) in [("config.json", &cfg), ("tokenizer.json", &tok)] {
        if dest.exists() {
            continue;
        }
        if let Some(p) = progress {
            p(&format!("  fetching {name}"));
        }
        download_hf_file(model_id, "main", name, dest, progress)?;
    }

    let weights = if weights_st.exists() {
        weights_st
    } else if weights_pt.exists() {
        weights_pt
    } else {
        // Try safetensors first (faster mmap, smaller); fall back to .bin.
        if let Some(p) = progress {
            p("  fetching model.safetensors");
        }
        if download_hf_file(model_id, "main", "model.safetensors", &weights_st, progress).is_ok() {
            weights_st
        } else {
            if let Some(p) = progress {
                p("  fetching pytorch_model.bin");
            }
            download_hf_file(model_id, "main", "pytorch_model.bin", &weights_pt, progress)?;
            weights_pt
        }
    };

    Ok((cfg, tok, weights))
}

/// Download `<model_id>/resolve/<rev>/<filename>` from huggingface.co into
/// `dest`. Follows redirects manually so we can resolve the relative
/// `Location:` header that the HF cache emits.
fn download_hf_file(
    model_id: &str,
    revision: &str,
    filename: &str,
    dest: &Path,
    progress: Option<&dyn Fn(&str)>,
) -> Result<()> {
    const BASE: &str = "https://huggingface.co";
    let initial = format!("{BASE}/{model_id}/resolve/{revision}/{filename}");
    let agent = ureq::AgentBuilder::new().redirects(0).build();

    let mut url = initial;
    for _ in 0..10 {
        let resp = agent
            .get(&url)
            .call()
            .map_err(|e| Error::embedder(format!("GET {url}: {e}")))?;
        let status = resp.status();
        if (300..400).contains(&status) {
            let loc = resp
                .header("location")
                .ok_or_else(|| Error::embedder(format!("{url}: redirect with no Location")))?;
            url = if loc.starts_with("http") {
                loc.to_string()
            } else if loc.starts_with('/') {
                format!("{BASE}{loc}")
            } else {
                // Relative-to-current-URL: pop trailing path segment, then join.
                let cut = url.rfind('/').unwrap_or(url.len());
                format!("{}/{}", &url[..cut], loc)
            };
            continue;
        }
        if !(200..300).contains(&status) {
            return Err(Error::embedder(format!("{url}: HTTP {status}")));
        }
        let total: Option<u64> = resp.header("content-length").and_then(|s| s.parse().ok());
        if let (Some(total), Some(p)) = (total, progress) {
            p(&format!("  {filename}: {} bytes", total));
        }
        let tmp = dest.with_extension("part");
        {
            let mut file = std::fs::File::create(&tmp)?;
            let mut reader = resp.into_reader();
            std::io::copy(&mut reader, &mut file)?;
        }
        std::fs::rename(&tmp, dest)?;
        return Ok(());
    }
    Err(Error::embedder(format!(
        "too many redirects for {filename}"
    )))
}

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, BufReader, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Copy)]
struct ModelSpec {
    label: &'static str,
    file_name: &'static str,
    url: &'static str,
    sha256: &'static str,
    size: &'static str,
}

const RECOGNITION_MODEL: ModelSpec = ModelSpec {
    label: "English recognition model",
    file_name: "ggml-base.en-q5_1.bin",
    size: "about 60 MB",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/c521a4b02f422512d734391fdf08bb08c0862f68/ggml-base.en-q5_1.bin",
    sha256: "4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f",
};
const VAD_MODEL: ModelSpec = ModelSpec {
    label: "Voice activity detection model",
    file_name: "ggml-silero-v6.2.0.bin",
    size: "about 1 MB",
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/9ffd54a1e1ee413ddf265af9913beaf518d1639b/ggml-silero-v6.2.0.bin",
    sha256: "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
};

pub(crate) struct ModelPaths {
    pub(crate) recognition: PathBuf,
    pub(crate) vad: PathBuf,
}

pub(crate) fn ensure_models() -> Result<ModelPaths> {
    let home = env::var_os("HOME").context("HOME is not set")?;
    let directory = PathBuf::from(home).join("Library/Caches/what-sorry/models");
    let targets = [RECOGNITION_MODEL, VAD_MODEL].map(|spec| (spec, directory.join(spec.file_name)));
    targets
        .iter()
        .filter(|(_, path)| path.exists())
        .try_for_each(|(spec, path)| verify_model(path, *spec))?;
    let missing = targets
        .iter()
        .filter(|(_, path)| !path.exists())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        confirm_download(&missing)?;
        missing
            .into_iter()
            .try_for_each(|(spec, path)| download_model(path, *spec))?;
    }
    Ok(ModelPaths {
        recognition: directory.join(RECOGNITION_MODEL.file_name),
        vad: directory.join(VAD_MODEL.file_name),
    })
}

fn confirm_download(missing: &[&(ModelSpec, PathBuf)]) -> Result<()> {
    if !io::stdin().is_terminal() {
        bail!(
            "Models are missing. Run What, Sorry? from an interactive terminal to download them."
        );
    }
    let names = missing
        .iter()
        .map(|(spec, _)| format!("{} ({})", spec.label, spec.size))
        .collect::<Vec<_>>()
        .join(", ");
    eprint!("Download {names}? [y/N] ");
    io::stderr()
        .flush()
        .context("Could not flush model prompt")?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("Could not read model download confirmation")?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        bail!("Model download cancelled")
    }
}

fn download_model(path: &Path, spec: ModelSpec) -> Result<()> {
    let parent = path.parent().context("Model cache has no parent")?;
    fs::create_dir_all(parent).context("Could not create model cache")?;
    let partial = path.with_extension("bin.part");
    let status = Command::new("/usr/bin/curl")
        .args(["--fail", "--location", "--progress-bar", "--output"])
        .arg(&partial)
        .arg(spec.url)
        .status()
        .context("Could not start curl")?;
    if !status.success() {
        let _ = fs::remove_file(&partial);
        bail!("Could not download {}", spec.label);
    }
    verify_model(&partial, spec).inspect_err(|_| {
        let _ = fs::remove_file(&partial);
    })?;
    fs::rename(partial, path).context("Could not move verified model into cache")
}

fn verify_model(path: &Path, spec: ModelSpec) -> Result<()> {
    let file =
        fs::File::open(path).with_context(|| format!("Could not open {}", path.display()))?;
    verify_sha256(BufReader::new(file), spec.sha256)
        .with_context(|| format!("Model checksum is invalid: {}", path.display()))
}

fn verify_sha256(mut reader: impl Read, expected: &str) -> Result<()> {
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .context("Could not read model checksum input")?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let actual = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual == expected {
        Ok(())
    } else {
        bail!("SHA-256 mismatch")
    }
}

#[cfg(test)]
mod tests {
    use super::verify_sha256;
    use anyhow::Result;
    #[test]
    fn accepts_matching_checksum() -> Result<()> {
        // GIVEN
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        // WHEN
        let actual = verify_sha256(b"abc".as_slice(), expected);
        // THEN
        assert!(actual.is_ok());
        Ok(())
    }
}

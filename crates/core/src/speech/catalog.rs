//! What can be brought in: whisper.cpp and the models it hears with, each
//! pinned by size and SHA-256.

/// A file of a known size and content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinned {
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

/// A model that hears Japanese.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechModel {
    pub name: String,
    /// How fast it hears, beside the others.
    pub speed: String,
    /// How well it hears, beside the others.
    pub accuracy: String,
    pub file: Pinned,
}

/// The versions Famulen brings in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    pub models: Vec<SpeechModel>,
    /// The release of whisper.cpp.
    pub version: String,
    /// The official builds, and the source for where there is none.
    pub linux_x64: Pinned,
    pub linux_arm64: Pinned,
    pub source: Pinned,
    /// CMake for macOS, to build the source with where there is none.
    pub cmake_macos: Pinned,
}

fn pinned(url: impl Into<String>, size: u64, sha256: &str) -> Pinned {
    Pinned {
        url: url.into(),
        size,
        sha256: sha256.to_owned(),
    }
}

const MODELS: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1";
const RELEASES: &str = "https://github.com/ggml-org/whisper.cpp/releases/download/b5907";

fn model(name: &str, speed: &str, accuracy: &str, size: u64, sha256: &str) -> SpeechModel {
    SpeechModel {
        name: name.to_owned(),
        speed: speed.to_owned(),
        accuracy: accuracy.to_owned(),
        file: pinned(format!("{MODELS}/ggml-{name}.bin"), size, sha256),
    }
}

impl Catalog {
    pub fn pinned() -> Self {
        Self {
            models: vec![
                model(
                    "large-v3-turbo",
                    "遅め",
                    "高い",
                    1_624_555_275,
                    "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
                ),
                model(
                    "large-v3-turbo-q8_0",
                    "遅め",
                    "高い",
                    874_188_075,
                    "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
                ),
                model(
                    "large-v3-turbo-q5_0",
                    "遅め",
                    "やや高い",
                    574_041_195,
                    "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
                ),
                model(
                    "small",
                    "速い",
                    "中くらい",
                    487_601_967,
                    "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
                ),
                model(
                    "base",
                    "とても速い",
                    "低め",
                    147_951_465,
                    "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
                ),
            ],
            version: "1.9.6".to_owned(),
            linux_x64: pinned(
                format!("{RELEASES}/whisper-bin-ubuntu-x64.tar.gz"),
                10_413_708,
                "a6199027fe5b4239c84171d2bca4c0ff44ec86a515494d5a39b237b212d429bd",
            ),
            linux_arm64: pinned(
                format!("{RELEASES}/whisper-bin-ubuntu-arm64.tar.gz"),
                4_623_550,
                "da2e86ac058e1db0281167355428665b82c0d92e4506d186a3894bec55ceefef",
            ),
            source: pinned(
                "https://github.com/ggml-org/whisper.cpp/archive/refs/tags/v1.9.6.tar.gz",
                10_064_959,
                "7ab15dc88022d2425ad94dcffd8abbf1b6d7018089b4e48d6b40780738e0b445",
            ),
            cmake_macos: pinned(
                "https://github.com/Kitware/CMake/releases/download/v4.5.1/cmake-4.5.1-macos-universal.tar.gz",
                92_792_874,
                "e1fb288543e4eb2a410e385ecebc64323bda51f0205782f2b094ccd5d572ca2d",
            ),
        }
    }

    pub fn model(&self, name: impl AsRef<str>) -> Option<&SpeechModel> {
        self.models.iter().find(|m| m.name == name.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_hears_japanese_and_every_file_is_pinned() {
        let catalog = Catalog::pinned();
        assert!(catalog.models.iter().all(|m| !m.name.contains(".en")));
        let files = catalog.models.iter().map(|m| &m.file).chain([
            &catalog.linux_x64,
            &catalog.linux_arm64,
            &catalog.source,
            &catalog.cmake_macos,
        ]);
        for file in files {
            assert!(file.url.starts_with("https://"), "{}", file.url);
            assert!(file.size > 0, "{}", file.url);
            assert_eq!(file.sha256.len(), 64, "{}", file.url);
            assert!(
                file.sha256.chars().all(|c| c.is_ascii_hexdigit()),
                "{}",
                file.url
            );
            assert!(!file.url.contains("/main/"), "{}", file.url);
        }
    }
}

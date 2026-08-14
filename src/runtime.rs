use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::{
    apk,
    dalvik::{execute, DexFile, DexHeader, ExecutionResult, Registers},
    framework::{ActivityManager, Intent},
    gles::GlesContext,
    manifest::AppManifest,
    resources::ResourceTable,
    VirtualScreen,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub screen: VirtualScreen,
    pub api_level: u32,
    pub release: &'static str,
    pub max_steps: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            screen: VirtualScreen::default(),
            api_level: 4,
            release: "Donut",
            max_steps: 100_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchReport {
    pub package: String,
    pub dex: String,
    pub launcher_activity: String,
    pub message: String,
}

pub struct Runtime {
    pub config: RuntimeConfig,
    pub activities: ActivityManager,
    pub graphics: GlesContext,
}

impl Default for Runtime {
    fn default() -> Self {
        let config = RuntimeConfig::default();
        Self {
            graphics: GlesContext::new(config.screen),
            config,
            activities: ActivityManager::default(),
        }
    }
}

impl Runtime {
    pub fn validate_apk(&self, path: impl AsRef<Path>) -> Result<LaunchReport> {
        let path = path.as_ref();
        let info = apk::inspect(path)?;
        if !info.has_manifest {
            bail!("APK has no AndroidManifest.xml");
        }
        if !info.has_dex {
            bail!("APK has no classes.dex");
        }
        let file = File::open(path).with_context(|| format!("open APK {}", path.display()))?;
        let mut archive = zip::ZipArchive::new(file)?;
        let manifest_bytes = read_entry(&mut archive, "AndroidManifest.xml")?;
        let dex_bytes = read_entry(&mut archive, "classes.dex")?;
        let manifest = AppManifest::parse_axml(&manifest_bytes)?;
        let dex = DexFile::parse(&dex_bytes)?;
        let resource_status = if info.entries.iter().any(|entry| entry == "resources.arsc") {
            match ResourceTable::parse(&read_entry(&mut archive, "resources.arsc")?) {
                Ok(table) => format!("{} resources", table.values.len()),
                Err(_) => "present but not decoded".to_owned(),
            }
        } else {
            "not present".to_owned()
        };
        let launcher = manifest
            .launcher_activity
            .clone()
            .unwrap_or_else(|| "none".to_owned());
        Ok(LaunchReport {
            package: manifest.package,
            launcher_activity: launcher.clone(),
            dex: format!(
                "{} bytes / {} classes / {} methods",
                dex.header.file_size,
                dex.classes.len(),
                dex.methods.len()
            ),
            message: format!(
                "manifest decoded; launcher: {launcher}; resources: {resource_status}"
            ),
        })
    }

    pub fn launch(&mut self, path: impl AsRef<Path>) -> Result<LaunchReport> {
        let path = path.as_ref();
        let report = self.validate_apk(path)?;
        let file = File::open(path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        let dex = DexFile::parse(&read_entry(&mut archive, "classes.dex")?)?;
        let activity = report.launcher_activity.clone();
        if activity == "none" {
            bail!("APK has no launcher activity");
        }
        let class_name = format!("L{};", activity.replace('.', "/"));
        self.activities.start_activity(
            activity.clone(),
            Intent {
                action: Some("android.intent.action.MAIN".to_owned()),
                categories: vec!["android.intent.category.LAUNCHER".to_owned()],
                component: Some(activity.clone()),
            },
        );
        if let Some(code) = dex.method_code(&class_name, "onCreate") {
            let mut registers = Registers::new(code.registers_size as usize);
            match execute(code, &mut registers, self.config.max_steps)? {
                ExecutionResult::ReturnVoid
                | ExecutionResult::Return(_)
                | ExecutionResult::Continue => {}
            }
        }
        Ok(report)
    }

    pub fn parse_dex(&self, bytes: &[u8]) -> Result<DexHeader> {
        DexHeader::parse(bytes)
    }
    pub fn parse_manifest(&self, bytes: &[u8]) -> Result<AppManifest> {
        AppManifest::parse_axml(bytes)
    }

    pub fn launch_plan(&self, path: impl AsRef<Path>) -> Result<LaunchPlan> {
        let path = path.as_ref();
        let info = apk::inspect(path)?;
        if !info.has_manifest {
            bail!("APK has no AndroidManifest.xml");
        }
        if !info.has_dex {
            bail!("APK has no classes.dex");
        }
        let file = File::open(path).with_context(|| format!("open APK {}", path.display()))?;
        let mut archive = zip::ZipArchive::new(file)?;
        let manifest = read_manifest(&mut archive)?;
        let dex = read_dex(&mut archive)?;
        let activity = manifest
            .launcher_activity
            .clone()
            .ok_or_else(|| anyhow::anyhow!("manifest has no MAIN/LAUNCHER activity"))?;
        let class_name = format!("L{};", activity.replace('.', "/"));
        let entry_method = dex.method_code(&class_name, "onCreate").is_some();
        Ok(LaunchPlan {
            package: manifest.package,
            activity,
            class_name,
            dex,
            entry_method,
        })
    }

    pub fn boot(&self, plan: &LaunchPlan) -> Result<BootState> {
        let code = plan
            .dex
            .method_code(&plan.class_name, "onCreate")
            .ok_or_else(|| anyhow::anyhow!("launcher onCreate() has no executable code"))?;
        let mut registers = Registers::new(code.registers_size as usize);
        let result = execute(code, &mut registers, 100_000)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let mut activities = ActivityManager::default();
        activities.start_activity(
            plan.activity.clone(),
            Intent {
                action: Some("android.intent.action.MAIN".to_owned()),
                categories: vec!["android.intent.category.LAUNCHER".to_owned()],
                component: Some(plan.activity.clone()),
            },
        );
        Ok(BootState { result, activities })
    }
}

#[derive(Debug)]
pub struct LaunchPlan {
    pub package: String,
    pub activity: String,
    pub class_name: String,
    pub dex: DexFile,
    pub entry_method: bool,
}

#[derive(Debug)]
pub struct BootState {
    pub result: ExecutionResult,
    pub activities: ActivityManager,
}

fn read_manifest(archive: &mut zip::ZipArchive<File>) -> Result<AppManifest> {
    let mut entry = archive.by_name("AndroidManifest.xml")?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    AppManifest::parse_axml(&bytes)
}

fn read_dex(archive: &mut zip::ZipArchive<File>) -> Result<DexFile> {
    let mut entry = archive.by_name("classes.dex")?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    DexFile::parse(&bytes)
}

fn read_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>> {
    let mut entry = archive
        .by_name(name)
        .with_context(|| format!("missing APK entry {name}"))?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}

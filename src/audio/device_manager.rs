use cpal::{Device, Host};
use cpal::traits::{DeviceTrait, HostTrait};

#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_default: bool,
}

pub struct AudioDeviceManager {
    host: Host,
    devices: Vec<AudioDeviceInfo>,
}

impl AudioDeviceManager {
    pub fn new() -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let mut manager = AudioDeviceManager {
            host,
            devices: Vec::new(),
        };
        
        if let Err(e) = manager.refresh_devices() {
            log::error!("Failed to enumerate audio devices: {}", e);
            return Err(anyhow::anyhow!("Failed to enumerate audio devices: {}", e));
        }
        
        Ok(manager)
    }
    
    pub fn refresh_devices(&mut self) -> anyhow::Result<()> {
        self.devices.clear();
        
        // Get default output device name
        let default_device_name = self.host.default_output_device()
            .and_then(|device| device.name().ok());
        
        // Enumerate all output devices
        let devices = self.host.output_devices()
            .map_err(|e| {
                log::error!("Failed to enumerate output devices: {}", e);
                anyhow::anyhow!("Failed to enumerate output devices: {}", e)
            })?;
        
        for device in devices {
            match device.name() {
                Ok(name) => {
                    let is_default = default_device_name.as_ref() == Some(&name);
                    self.devices.push(AudioDeviceInfo {
                        name: name.clone(),
                        is_default,
                    });
                    log::debug!("Found audio device: {} (default: {})", name, is_default);
                }
                Err(e) => {
                    log::warn!("Failed to get device name: {}", e);
                }
            }
        }
        
        log::info!("Enumerated {} audio output devices", self.devices.len());
        Ok(())
    }
    
    pub fn get_devices(&self) -> &[AudioDeviceInfo] {
        &self.devices
    }
    
    pub fn get_device_by_name(&self, name: &str) -> anyhow::Result<Device> {
        let devices = self.host.output_devices()
            .map_err(|e| {
                log::error!("Failed to enumerate devices when searching for '{}': {}", name, e);
                anyhow::anyhow!("Failed to enumerate devices: {}", e)
            })?;
        
        for device in devices {
            if let Ok(device_name) = device.name() {
                if device_name == name {
                    log::debug!("Found requested audio device: {}", name);
                    return Ok(device);
                }
            }
        }
        
        log::warn!("Audio device '{}' not found, falling back to default", name);
        self.get_default_device()
    }
    
    pub fn get_default_device(&self) -> anyhow::Result<Device> {
        self.host.default_output_device()
            .ok_or_else(|| {
                log::error!("No default audio output device available");
                anyhow::anyhow!("No default audio output device available")
            })
    }
}

// Jackson Coxson

use std::{
    fs::File,
    io::{stdin, Read, Seek, Write},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use dialoguer::{theme::ColorfulTheme, Select};
use plist_plus::PlistType;
use rusty_libimobiledevice::{idevice::Device, services::userpref};
use walkdir::WalkDir;
use zip::{result::ZipError, write::FileOptions};

fn main() {
    _main();
    println!("Press any key to exit...");
    stdin().read_line(&mut String::new()).unwrap();
}

fn _main() {
    println!(
        r#"欢迎使用 SideStore 下载器
        你将会被此程序引导完成下载 SideStore.ipa 并为你的设备修改的步骤
        请确保你的设备使用 USB 或者网络连接,以便我们从中提取所需信息
        "#
    );
    #[cfg(target_os = "macos")]
    println!("确保打开 Finder 并且设备显示在侧栏上");
    #[cfg(target_os = "windows")]
    println!("确保已经安装 itunes 并且允许连接");
    #[cfg(target_os = "linux")]
    println!("确保 usbmuxd 已安装并正在运行，Ubuntu 可以从 apt 安装它");

    println!("步骤 1/7: 下载 SideStore .ipa");
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("我们将会下载最新的稳定版 ipa 你是否要指定其他 URL？")
        .default(0)
        .items(&[ "不 (推荐)", "是", "选择本地 ipa"])
        .interact()
        .unwrap();

    let ipa_bytes = if selection == 0 || selection == 1 {
        let url = if selection == 0 {
            "https://github.com/SideStore/SideStore/releases/latest/download/SideStore.ipa"
                .to_string()
        } else {
            println!("请输入 SideStore 的文件地址(以 .ipa 结尾)");
            let mut s = String::new();
            stdin()
                .read_line(&mut s)
                .expect("未输入正确的字符串");

            s.trim().to_string()
        };

        let agent = ureq::AgentBuilder::new()
            .tls_connector(Arc::new(native_tls::TlsConnector::new().unwrap()))
            .build();

        let ipa_bytes = match agent.get(&url).call() {
            Ok(i) => i,
            Err(e) => {
                println!("无法从指定的URL下载： {:?}", e);
                return;
            }
        };
        let mut x_vec = Vec::new();
        if ipa_bytes.into_reader().read_to_end(&mut x_vec).is_err() {
            println!("从 URL 获取文件字节时出错");
            return;
        }
        x_vec
    } else {
        println!("输入 SideStore .ipa 的路径");
        let mut s = String::new();
        stdin()
            .read_line(&mut s)
            .expect("未输入正确的字符串");

        let path = Path::new(s.trim());
        let mut file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                println!("无法打开文件: {:?}", e);
                return;
            }
        };
        let mut x_vec = Vec::new();
        if file.read_to_end(&mut x_vec).is_err() {
            println!("无法从文件取得字节");
            return;
        }
        x_vec
    };
    let cursor = std::io::Cursor::new(ipa_bytes);
    let mut archive = match zip::read::ZipArchive::new(cursor) {
        Ok(a) => a,
        Err(e) => {
            println!("无法将下载内容转换为存档： {:?}", e);
            return;
        }
    };

    println!("\n\n步骤 2/7: 选择目录保存 .ipa");
    println!("现在输入路径(使用 . 选择程序运行目录)");
    let mut s = String::new();
    stdin()
        .read_line(&mut s)
        .expect("未输入正确的字符串");
    if s.trim() == "." {
        // nightmare nightmare nightmare nightmare nightmare nightmare nightmare nightmare nightmare
        s = std::env::current_dir()
            .unwrap()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
    }
    let save_path = match PathBuf::from_str(s.trim()) {
        Ok(t) => t,
        Err(e) => {
            println!("路径字符串错误: {:?}", e);
            return;
        }
    };
    if !save_path.exists() {
        match std::fs::create_dir_all(&save_path) {
            Ok(_) => (),
            Err(e) => {
                println!("找不到路径，创建失败： {:?}", e);
                return;
            }
        }
    }

    println!("\n\n步骤 3/7: 选择你的设备并为其创建 .ipa 文件");
    let device;
    let device_name;
    loop {
        let devices = match rusty_libimobiledevice::idevice::get_devices() {
            Ok(d) => d,
            Err(e) => {
                println!("无法获取设备列表： {:?}", e);
                #[cfg(target_os = "windows")]
                println!("请确保 iTunes 已经安装并且打开");
                #[cfg(target_os = "linux")]
                println!("请确保 usbmuxd 正在运行");
                continue;
            }
        };
        if devices.is_empty() {
            println!("没有设备连接");
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }
        if devices.len() == 1 {
            // That clone might be the end of me
            device = devices[0].clone();
            let lock_cli = match device.new_lockdownd_client("ss_downloader") {
                Ok(l) => l,
                Err(e) => {
                    println!("无法在设备上启动 lockdownd 客户端： {:?}", e);
                    return;
                }
            };
            let name = match lock_cli.get_device_name() {
                Ok(n) => n,
                Err(e) => {
                    println!("无法获取设备名称: {:?}", e);
                    return;
                }
            };
            println!("使用唯一连接的设备: {}", name);
            device_name = name;
            break;
        }
        let mut mp = Vec::with_capacity(devices.len());
        for device in devices {
            let lock_cli = match device.new_lockdownd_client("ss_downloader") {
                Ok(l) => l,
                Err(_) => continue,
            };
            let name = match lock_cli.get_device_name() {
                Ok(n) => n,
                Err(_) => continue,
            };
            // I hate this, but I'm lazy
            mp.push((
                device.clone(),
                format!(
                    "{} ({})",
                    name,
                    if device.get_network() {
                        "Network"
                    } else {
                        "USB"
                    }
                ),
            ));
        }

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("选择你的设备")
            .default(0)
            .items(
                &mp.iter()
                    // Send help
                    .map(|x| x.1.clone())
                    .collect::<Vec<String>>()
                    .to_vec(),
            )
            .interact()
            .unwrap();

        // Talk about emotional damage
        device = mp[selection].0.clone();
        device_name = mp[selection].1.clone();
        break;
    }
    let device_name = device_name
        .replace(' ', "_")
        .replace('(', "")
        .replace(')', "")
        .replace('’', "")
        .replace('\'', "");

    println!("\n\n步骤 4/7: 检查配对文件");
    if device.get_network() {
        println!("设备通过网络连接，跳过测试");
    } else {
        println!("强烈建议您测试设备的配对文件");
        println!("这将确保 SideStore 能够使用它");
        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("是否继续进行配对文件测试？")
            .default(0)
            .items(&["是 (墙裂推荐)", "不"])
            .interact()
            .unwrap();
        match selection {
            0 => {
                println!("输入设备的本地 IP 地址");
                let mut s = String::new();
                stdin().read_line(&mut s).expect("终止");
                let ip = match std::net::IpAddr::from_str(s.trim()) {
                    Ok(i) => i,
                    Err(e) => {
                        println!("无法解析输入： {:?}", e);
                        return;
                    }
                };

                loop {
                    if test_device(ip, device.get_udid()) {
                        break;
                    }

                    // Unwrapping because we already know this works
                    let lock_cli = device.new_lockdownd_client("ss_downloader_regen").unwrap();
                    if let Err(e) = lock_cli.set_value(
                        "EnableWifiDebugging".to_string(),
                        "com.apple.mobile.wireless_lockdown".to_string(),
                        true.into(),
                    ) {
                        println!("无法启动 WiFi 同步: {:?}", e);
                        println!("请确保您已设置密码");
                        return;
                    }
                    if test_device(ip, device.get_udid()) {
                        break;
                    }
                    if let Err(e) = lock_cli.pair(None, None) {
                        println!("无法与您的设备配对 {:?}", e);
                        return;
                    }
                }
            }
            1 => {
                println!("跳过测试，无法保证 SideStore 将正常工作")
            }
            _ => unreachable!(),
        }
    }

    println!("\n\n步骤 5/6: 提取并修改 .ipa");
    if archive.extract(&save_path.join("temp")).is_err() {
        println!("无法解压缩归档文件");
        return;
    }

    let plist_path = save_path
        .join("temp")
        .join("Payload")
        .join("SideStore.app")
        .join("Info.plist");
    if !plist_path.exists() {
        println!("存档不包含 Info.plist");
        return;
    }

    let mut buf = Vec::new();
    let mut plist_file = std::fs::File::open(&plist_path).unwrap();
    plist_file.read_to_end(&mut buf).unwrap();
    let mut info_plist = match plist_plus::Plist::from_bin(buf[..].to_vec()) {
        Ok(i) => i,
        Err(e) => {
            println!("无法从文件读取plist: {:?}", e);
            return;
        }
    };

    if info_plist.plist_type != PlistType::Dictionary {
        println!("Info.plist 格式不正确");
        return;
    }

    info_plist
        .dict_set_item("ALTDeviceID", device.get_udid().into())
        .unwrap();

    let pairing_file = match userpref::read_pair_record(device.get_udid()) {
        Ok(mut p) => {
            p.dict_set_item("UDID", device.get_udid().into()).unwrap();
            p.to_string()
        }
        Err(e) => {
            println!("无法读取设备的配对文件： {:?}", e);
            return;
        }
    };

    info_plist
        .dict_set_item("ALTPairingFile", pairing_file.into())
        .unwrap();

    let info_plist = info_plist.to_string();
    std::fs::remove_file(&plist_path).unwrap();
    let mut f = std::fs::File::create(plist_path).unwrap();
    let _ = f.write(info_plist.as_bytes()).unwrap();

    println!("\n\n步骤 6/6: 压缩 ipa");
    pls_zip(
        save_path.join("temp").to_str().unwrap(),
        save_path
            .join(format!("SideStore-{}.ipa", device_name))
            .to_str()
            .unwrap(),
        zip::CompressionMethod::Deflated,
    )
    .unwrap();
    std::fs::remove_dir_all(save_path.join("temp")).unwrap();

    println!("\n\n大功告成!不要与他人共享此 ipa，它包含您设备的私人信息");
}

fn test_device(ip: std::net::IpAddr, udid: String) -> bool {
    let device = Device::new(udid, Some(ip), 696969);
    match device.new_heartbeat_client("ss_downloader_tester") {
        Ok(_) => true,
        Err(e) => {
            println!("测试失败! {:?}", e);
            false
        }
    }
}

fn zip_dir<T>(
    it: &mut dyn Iterator<Item = walkdir::DirEntry>,
    prefix: &str,
    writer: T,
    method: zip::CompressionMethod,
) -> zip::result::ZipResult<()>
where
    T: Write + Seek,
{
    let mut zip = zip::ZipWriter::new(writer);
    let options = FileOptions::default()
        .compression_method(method)
        .unix_permissions(0o755);

    let mut buffer = Vec::new();
    for entry in it {
        let path = entry.path();
        let name = path.strip_prefix(Path::new(prefix)).unwrap();

        // Write file or directory explicitly
        // Some unzip tools unzip files with directory paths correctly, some do not!
        if path.is_file() {
            #[allow(deprecated)]
            zip.start_file_from_path(name, options)?;
            let mut f = File::open(path)?;

            f.read_to_end(&mut buffer)?;
            zip.write_all(&*buffer)?;
            buffer.clear();
        } else if !name.as_os_str().is_empty() {
            // Only if not root! Avoids path spec / warning
            // and mapname conversion failed error on unzip
            #[allow(deprecated)]
            zip.add_directory_from_path(name, options)?;
        }
    }
    zip.finish()?;
    Result::Ok(())
}

fn pls_zip(
    src_dir: &str,
    dst_file: &str,
    method: zip::CompressionMethod,
) -> zip::result::ZipResult<()> {
    if !Path::new(src_dir).is_dir() {
        return Err(ZipError::FileNotFound);
    }

    let path = Path::new(dst_file);
    let file = File::create(&path).unwrap();

    let walkdir = WalkDir::new(src_dir);
    let it = walkdir.into_iter();

    zip_dir(&mut it.filter_map(|e| e.ok()), src_dir, file, method)?;

    Ok(())
}

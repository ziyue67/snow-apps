use ffmpeg_sidecar::{child::FfmpegChild, command::FfmpegCommand, event::FfmpegEvent};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{io::Result, path::PathBuf};
use tauri::{Manager, path::BaseDirectory};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Copy)]
pub enum VideoRecordState {
    Idle,
    Recording,
    Paused,
}

#[derive(PartialEq, Serialize, Deserialize, Debug, Clone, Copy)]
pub enum VideoFormat {
    Mp4,
    Gif,
}

impl VideoFormat {
    pub fn extension(&self) -> &str {
        match self {
            VideoFormat::Mp4 => "mp4",
            VideoFormat::Gif => "gif",
        }
    }
}

// 录制参数结构体，用于在暂停后恢复录制时重用参数
#[derive(Clone)]
struct RecordingParams {
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
    output_file: String,
    format: VideoFormat,
    frame_rate: u32,
    enable_microphone: bool,
    enable_system_audio: bool,
    microphone_device_name: String,
    hwaccel: bool,
    encoder: String,
    encoder_preset: String,
}

pub struct VideoRecordService {
    pub state: VideoRecordState,
    pub child: Option<FfmpegChild>,
    // 片段管理相关字段
    segments: Vec<String>,                     // 存储所有片段文件路径
    segment_counter: u32,                      // 片段计数器
    recording_params: Option<RecordingParams>, // 录制参数，用于恢复录制
    ffmpeg_path: Option<PathBuf>,
}

impl VideoRecordService {
    pub fn new() -> Self {
        Self {
            state: VideoRecordState::Idle,
            child: None,
            segments: Vec::new(),
            segment_counter: 0,
            recording_params: None,
            ffmpeg_path: None,
        }
    }

    pub fn init(&mut self, app: &tauri::AppHandle) {
        if self.ffmpeg_path.is_none() {
            let resource_path = match app.path().resolve("ffmpeg", BaseDirectory::Resource) {
                Ok(resource_path) => resource_path,
                Err(_) => panic!("[VideoRecordService] Failed to get resource path"),
            };

            #[cfg(target_os = "windows")]
            let ffmpeg_name = "ffmpeg.exe";
            #[cfg(not(target_os = "windows"))]
            let ffmpeg_name = "ffmpeg";

            let resource_ffmpeg = resource_path.join(ffmpeg_name);
            self.ffmpeg_path = Some(if resource_ffmpeg.exists() {
                resource_ffmpeg
            } else {
                PathBuf::from(ffmpeg_name)
            });
        }
    }

    pub fn get_ffmpeg_command(&self) -> FfmpegCommand {
        FfmpegCommand::new_with_path(
            self.ffmpeg_path
                .as_ref()
                .expect("[VideoRecordService] valid ffmpeg path"),
        )
    }

    pub fn start(
        &mut self,
        min_x: i32,
        min_y: i32,
        max_x: i32,
        max_y: i32,
        output_file: String,
        format: VideoFormat,
        frame_rate: u32,
        enable_microphone: bool,
        enable_system_audio: bool,
        microphone_device_name: String,
        hwaccel: bool,
        encoder: String,
        encoder_preset: String,
    ) -> Result<()> {
        if self.state == VideoRecordState::Recording {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Recording is already in progress",
            ));
        }

        // 保存录制参数
        self.recording_params = Some(RecordingParams {
            min_x,
            min_y,
            max_x,
            max_y,
            output_file: output_file.clone(),
            format,
            frame_rate,
            enable_microphone,
            enable_system_audio,
            microphone_device_name,
            hwaccel,
            encoder,
            encoder_preset,
        });

        // 重置片段相关状态
        self.segments.clear();
        self.segment_counter = 0;

        // 开始第一个片段的录制
        self.start_segment()
    }

    fn start_segment(&mut self) -> Result<()> {
        let params = self.recording_params.as_ref().unwrap();

        // 计算录制区域的宽度和高度
        let mut width = params.max_x - params.min_x;
        let mut height = params.max_y - params.min_y;

        if width <= 0 || height <= 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid recording area dimensions",
            ));
        }

        // 确保宽度和高度都是偶数（libx264要求）
        if width % 2 == 1 {
            width -= 1;
        }
        if height % 2 == 1 {
            height -= 1;
        }

        println!(
            "Recording segment {} area: {}x{} at ({}, {})",
            self.segment_counter + 1,
            width,
            height,
            params.min_x,
            params.min_y
        );

        let mut command = self.get_ffmpeg_command();

        // 硬件加速选项必须在输入选项之前
        if params.hwaccel {
            command.arg("-hwaccel").arg("auto");
        }

        #[cfg(target_os = "windows")]
        {
            command
                .arg("-f")
                .arg("gdigrab")
                .arg("-framerate")
                .arg(params.frame_rate.to_string())
                .arg("-offset_x")
                .arg(params.min_x.to_string())
                .arg("-offset_y")
                .arg(params.min_y.to_string())
                .arg("-video_size")
                .arg(format!("{}x{}", width, height))
                .arg("-i")
                .arg("desktop");
        }

        #[cfg(not(target_os = "windows"))]
        {
            command
                .arg("-f")
                .arg("x11grab")
                .arg("-framerate")
                .arg(params.frame_rate.to_string())
                .arg("-video_size")
                .arg(format!("{}x{}", width, height))
                .arg("-i")
                .arg(format!(":0.0+{},{}", params.min_x, params.min_y));
        }

        // 音频输入计数器
        let mut audio_inputs: Vec<String> = Vec::new();

        // 添加系统音频输入
        if params.enable_system_audio {
            // command
            //     .arg("-f")
            //     .arg("dshow")
            //     .arg("-i")
            //     .arg("audio=virtual-audio-capturer");
            // audio_inputs.push("1:a".to_string());
        }

        // 添加麦克风音频输入
        if params.enable_microphone {
            #[cfg(target_os = "windows")]
            {
                let device_names = self.get_microphone_device_names();

                if device_names.len() > 0 {
                    command.arg("-f").arg("dshow").arg("-i").arg(format!(
                        "audio={}",
                        if device_names.contains(&params.microphone_device_name) {
                            params.microphone_device_name.clone()
                        } else {
                            device_names[0].clone()
                        }
                    ));
                    audio_inputs.push(format!("{}:a", audio_inputs.len() + 1));
                }
            }

            #[cfg(not(target_os = "windows"))]
            {
                command.arg("-f").arg("pulse").arg("-i").arg("default");
                audio_inputs.push(format!("{}:a", audio_inputs.len() + 1));
            }
        }

        // 生成当前片段的文件名
        let segment_filename = format!(
            "{}_segment_{:03}.{}",
            params.output_file,
            self.segment_counter,
            params.format.extension()
        );

        // 确保输出文件的目录存在
        if let Some(parent_dir) = std::path::Path::new(&segment_filename).parent() {
            if let Err(e) = std::fs::create_dir_all(parent_dir) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to create output directory: {}", e),
                ));
            }
        }

        // 根据格式设置不同的参数
        match params.format {
            VideoFormat::Mp4 => {
                command.arg("-c:v").arg(&params.encoder);

                // 根据编码器类型设置预设值
                if params.encoder.contains("amf") {
                    // AMD AMF编码器只支持特定的预设值
                    let amf_preset = match params.encoder_preset.as_str() {
                        "ultrafast" | "superfast" | "veryfast" | "faster" | "fast" => "speed",
                        "medium" | "slow" => "balanced",
                        "slower" | "veryslow" | "placebo" => "quality",
                        // 如果已经是AMF支持的预设值，直接使用
                        "speed" | "balanced" | "quality" => &params.encoder_preset,
                        _ => "balanced", // 默认使用balanced
                    };
                    command.arg("-preset").arg(amf_preset);
                } else if params.encoder.contains("nvenc") {
                    // NVIDIA NVENC编码器支持的预设值
                    let nvenc_preset = match params.encoder_preset.as_str() {
                        "ultrafast" => "p1",              // 最快
                        "superfast" | "veryfast" => "p2", // 更快
                        "faster" | "fast" => "p3",        // 快
                        "medium" => "p4",                 // 中等（默认）
                        "slow" => "p5",                   // 慢
                        "slower" => "p6",                 // 更慢
                        "veryslow" | "placebo" => "p7",   // 最慢
                        // 如果已经是NVENC支持的预设值，直接使用
                        "p1" | "p2" | "p3" | "p4" | "p5" | "p6" | "p7" | "hq" | "hp" | "ll"
                        | "llhq" | "llhp" | "default" | "bd" | "lossless" | "losslesshp" => {
                            &params.encoder_preset
                        }
                        _ => "p4", // 默认使用p4（中等）
                    };
                    command.arg("-preset").arg(nvenc_preset);
                } else {
                    // 其他编码器（如x264）使用原始预设值
                    command.arg("-preset").arg(&params.encoder_preset);
                }

                command.arg("-crf").arg("23").arg("-pix_fmt").arg("yuv420p"); // 添加像素格式，确保兼容性

                // 音频编码设置
                if !audio_inputs.is_empty() {
                    command.arg("-c:a").arg("aac").arg("-b:a").arg("128k");

                    // 如果有多个音频输入，需要混音
                    if audio_inputs.len() > 1 {
                        let filter_complex = format!(
                            "[{}]amix=inputs={}[aout]",
                            audio_inputs.join("]["),
                            audio_inputs.len()
                        );
                        command.arg("-filter_complex").arg(filter_complex);
                        command.arg("-map").arg("0:v").arg("-map").arg("[aout]");
                    } else {
                        command
                            .arg("-map")
                            .arg("0:v")
                            .arg("-map")
                            .arg(&audio_inputs[0]);
                    }
                } else {
                    // 没有音频输入时，只映射视频
                    command.arg("-map").arg("0:v");
                }

                command.arg("-movflags").arg("+faststart"); // 优化MP4文件结构
            }
            VideoFormat::Gif => {
                // GIF格式不包含音频
                command
                    .arg("-vf")
                    .arg("fps=10,scale=-1:-1:flags=lanczos,palettegen=reserve_transparent=0")
                    .arg("-loop")
                    .arg("0");
            }
        }

        command.arg("-y");

        // 输出文件
        command.arg(&segment_filename);

        println!("FFmpeg segment command args: {:?}", command);

        // 启动ffmpeg进程
        match command.spawn() {
            Ok(mut child) => {
                for event in child.iter().unwrap() {
                    if params.format == VideoFormat::Mp4 {
                        match event {
                            FfmpegEvent::Progress(_) => {
                                self.child = Some(child);
                                self.state = VideoRecordState::Recording;
                                self.segments.push(segment_filename);
                                self.segment_counter += 1;
                                return Ok(());
                            }
                            _ => {}
                        }
                    }
                }

                Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Failed to start recording segment",
                ))
            }
            Err(e) => {
                self.state = VideoRecordState::Idle;
                println!("FFmpeg start error: {}", e);
                Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to start recording segment: {}", e),
                ))
            }
        }
    }

    pub fn get_microphone_device_names(&self) -> Vec<String> {
        #[cfg(not(target_os = "windows"))]
        {
            return vec!["default".to_string()];
        }

        #[cfg(target_os = "windows")]
        {
            let mut command = self.get_ffmpeg_command();
            command
                .arg("-list_devices")
                .arg("true")
                .arg("-f")
                .arg("dshow")
                .arg("-i")
                .arg("dummy");

            let mut device_names = Vec::new();

            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(e) => {
                    println!(
                        "[get_microphone_device_names] Failed to spawn ffmpeg: {}",
                        e
                    );
                    return device_names;
                }
            };

            let output_iter = match child.iter() {
                Ok(output) => output,
                Err(e) => {
                    println!("[get_microphone_device_names] Failed to iter ffmpeg: {}", e);
                    return device_names;
                }
            };

            // 创建正则表达式来匹配音频设备
            // 格式: [dshow @ address] [info] "设备名称" (audio)
            let device_regex = match Regex::new(r#"\[info\]\s+"([^"]+)"\s+\(audio\)"#) {
                Ok(regex) => regex,
                Err(e) => {
                    println!(
                        "[get_microphone_device_names] Failed to create regex: {}",
                        e
                    );
                    return device_names;
                }
            };

            for line in output_iter {
                match line {
                    FfmpegEvent::Log(_, line) => {
                        // 使用正则表达式解析音频设备
                        if let Some(captures) = device_regex.captures(&line) {
                            if let Some(device_name) = captures.get(1) {
                                let name = device_name.as_str().to_string();
                                device_names.push(name.clone());
                                println!(
                                    "[get_microphone_device_names] Found audio device: {}",
                                    name
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }

            let _ = child.wait();

            println!(
                "[get_microphone_device_names] Total found devices: {}",
                device_names.len()
            );
            device_names
        }
    }

    pub fn kill(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
        }

        self.cleanup();
        Ok(())
    }

    fn get_final_filename(&self) -> String {
        let params = self.recording_params.as_ref().unwrap();
        format!("{}.{}", params.output_file, params.format.extension())
    }

    pub fn stop(&mut self) -> Result<Option<String>> {
        if self.state != VideoRecordState::Recording && self.state != VideoRecordState::Paused {
            return Ok(None);
        }

        println!("[FFmpeg] Stopping and merging segments");

        // 停止当前录制
        if let Some(mut child) = self.child.take() {
            let _ = child.quit();
            let _ = child.wait();
        }

        // 如果只有一个片段，直接重命名
        let final_filename = self.get_final_filename();
        if self.segments.len() == 1 {
            if let Err(e) = std::fs::rename(&self.segments[0], &final_filename) {
                println!("Failed to rename single segment: {}", e);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to rename segment: {}", e),
                ));
            }
        } else if self.segments.len() > 1 {
            // 多个片段需要合并
            self.merge_segments(final_filename.clone())?;
        }

        self.cleanup();
        Ok(Some(final_filename))
    }

    fn merge_segments(&mut self, final_filename: String) -> Result<()> {
        let params = self.recording_params.as_ref().unwrap();

        // 创建临时的文件列表
        let list_filename = format!("{}_segments.txt", params.output_file);
        let mut list_content = String::new();

        for segment in &self.segments {
            list_content.push_str(&format!("file '{}'\n", segment));
        }

        if let Err(e) = std::fs::write(&list_filename, list_content) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create segment list: {}", e),
            ));
        }

        // 使用ffmpeg合并片段
        let mut command = self.get_ffmpeg_command();
        command
            .arg("-f")
            .arg("concat")
            .arg("-safe")
            .arg("0")
            .arg("-i")
            .arg(&list_filename)
            .arg("-c")
            .arg("copy")
            .arg("-y")
            .arg(&final_filename);

        println!("Merging segments with command: {:?}", command);

        match command.spawn() {
            Ok(mut child) => {
                let _ = child.wait();

                // 删除临时文件列表
                let _ = std::fs::remove_file(&list_filename);

                // 删除所有片段文件
                for segment in &self.segments {
                    if let Err(e) = std::fs::remove_file(segment) {
                        println!("Warning: Failed to delete segment file {}: {}", segment, e);
                    }
                }

                println!("Segments merged successfully");
                Ok(())
            }
            Err(e) => {
                println!("Failed to merge segments: {}", e);
                Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to merge segments: {}", e),
                ))
            }
        }
    }

    fn cleanup(&mut self) {
        self.state = VideoRecordState::Idle;
        self.segments.clear();
        self.segment_counter = 0;
        self.recording_params = None;
    }

    pub fn pause(&mut self) -> Result<()> {
        if self.state != VideoRecordState::Recording {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "No recording in progress",
            ));
        }

        println!("[FFmpeg] Pausing recording - stopping current segment");

        // 停止当前片段的录制
        if let Some(mut child) = self.child.take() {
            let _ = child.quit();
            let _ = child.wait();
        }

        self.state = VideoRecordState::Paused;
        Ok(())
    }

    pub fn resume(&mut self) -> Result<()> {
        if self.state != VideoRecordState::Paused {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Recording is not paused",
            ));
        }

        println!("[FFmpeg] Resuming recording - starting new segment");

        // 开始新片段的录制
        self.start_segment()
    }
}

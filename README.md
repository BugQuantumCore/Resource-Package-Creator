# MTR Resource Package Creator · MTR 资源包创建器

一个**独立于 Minecraft 运行**的桌面版 Minecraft Transit Railway（MTR）4.0.x 自定义资源包创建器，原生支持 Windows / Linux / macOS。以 Rust + C++ 编写（egui + wgpu 界面、C++ DSP 与原生脚本宿主），不依赖 Java 或 Minecraft 安装。

- 仓库：<https://github.com/BugQuantumCore/Resource-Package-Creator>
- 参考项目：[Minecraft-Transit-Railway](https://github.com/Minecraft-Transit-Railway/Minecraft-Transit-Railway) · [Transport-Simulation-Core](https://github.com/Minecraft-Transit-Railway/Transport-Simulation-Core) · [Joban-Client-Mod-Yanyang](https://github.com/BugQuantumCore/Joban-Client-Mod-Yanyang)（晏阳技术组）
- 语言：简体中文（中国大陆）、繁體中文（香港特別行政區）、繁體中文（台灣省）、文言（華夏）、English (US/UK)、Français (France)

## 功能

| 模块 | 说明 |
| --- | --- |
| **模型预览** | Blender 风格深色 3D 视口（轨道相机、网格、双面材质、半透明玻璃、选中高亮）。支持 OBJ + MTL、Blockbench `.bbmodel`，按 MTR 规则解析部件（`o`/`g` 分组）、贴图（`default.png` → 车辆贴图、同路径 png、`flipTextureV`）与材质（Kd/透明度）。 |
| **外部模型导入** | 「文件 → 导入模型」：Metasequoia `.mqo` / 压缩 `.mqoz`（材质 `col`/`tex`、`vertex`/`BVertex` UV、`face` 的 `V`/`M`/`UV`，左手坐标自动转右手、面序反转）与 glTF 2.0 `.gltf` / `.glb`（节点 TRS/矩阵变换烘焙、POSITION/NORMAL/TEXCOORD_0、索引、`baseColorFactor`→Kd、`baseColorTexture` 重编码 PNG、data URI 与外部文件）。导入后转为 OBJ+MTL+PNG 资源挂到车辆，可直接预览与导出。 |
| **开关门动画预览** | 时间轴完整复刻 TSC 停站门周期（`DOOR_DELAY = 1000 ms`、`DOOR_MOVE_TIME = 3200 ms`），并逐式移植 MTR 的 12 种门动画曲线（STANDARD…R211，含 `doorXMultiplier`/`doorZMultiplier`、`renderFrom/UntilOpeningDoorTime` 可见窗口）。可播放、暂停、拖拽 scrub，并实时驱动 3D 视口。 |
| **VVVF 试听** | 三种模式：① **参数合成器**（C++ 引擎：SPWM 自然采样 / SHE 特定谐波消去、同步脉冲带、V/f 与弱磁、RL 电机负载、直流阻断、滚噪）实时波形 + 频谱；② **BVE 电机数据 + 真实音源回放**（解析 `sound.cfg`（含 `[SoundList]`）与 `powervol/powerfreq/brakevol/brakefreq.csv` 或 BVE4 `train.dat`；自动加载音源集内的电机 WAV 循环——读取 RIFF PCM 8/16/24/32 位、IEEE float 32/64 位、WAVE_FORMAT_EXTENSIBLE——按频率表速率、音量表音量循环重采样，多声道叠加、增益平滑无咔哒声；无 WAV 时可用合成器近似或手动指定一个）；③ **旧版速度音**（MTR3 `_acceleration_Na` 命名与速度桶规则仿真）。均可输出 5 秒 WAV。 |
| **列车脚本（JCM 联动）** | 读写 `vehicleScripts`（晏阳技术组修改版 JCM 为 `mtr_custom_resources.json` 新增的设置项）：`scriptId`/`hideDisplayParts` 挂到车辆；`scriptLocations` 混排 JS 与 C++ 原生模块；`prependExpressions`/`input`(SCRIPT_INPUT)/`dataFetchMode`/`forceLoad` 全字段编辑。**JS 试运行**在 QuickJS 沙箱中以模拟列车数据执行 `create`/`render`/`dispose`；**C++ 模块**经 `mrpc_script.hpp`（`MRPC_REGISTER_VEHICLE_SCRIPT`）编译后由内置宿主 `dlopen` 预览。 |
| **编辑与打包** | 车辆全部字段（长度、宽度、转向架、风挡、贯通道、BVE/旧版音源……）、模型引用、部件（condition/renderStage/type/门动画/显示屏全参数）。校验器在导出前报告错误/警告；导出为符合原版结构的 zip（`pack.mcmeta`、`pack.png`、`assets/mtr/mtr_custom_resources.json`…），并保留未知字段与 MTR3 旧格式条目。 |
| **示例包** | 「文件 → 生成演示包」一键创建完全合法的示例车辆（程序化 OBJ/贴图/properties/positions/BVE 曲线/JS 脚本），亦可用 `cargo run -p mrpc-core --example gen_demo` 生成 zip 与试听 wav。 |

## 构建

需要 **Rust 1.82+** 与 **C++17 编译器**（g++/clang++/MSVC）。

```sh
# Linux 音频输出需要 ALSA 开发头（可选）
#   sudo apt install libasound2-dev
git clone https://github.com/BugQuantumCore/Resource-Package-Creator.git
cd Resource-Package-Creator
cargo run -p mrpc-app --release                # 桌面程序（默认无音频输出）
cargo run -p mrpc-app --release --features audio   # 带实时音频试听

cargo test --workspace                          # 54 个单元测试
cargo run -p mrpc-core --example gen_demo -- ./out   # 示例包 + VVVF wav
```

详见 [BUILDING.md](BUILDING.md)（各平台细节）与 [ARCHITECTURE.md](ARCHITECTURE.md)（模块设计）。

## 仓库布局

```
crates/
├── mrpc-core/    # 资源包数据模型、格式解析/生成/校验/打包、i18n（Rust）
├── mrpc-vvvf/    # VVVF 合成引擎（C++ DSP，extern "C" ABI）
├── mrpc-script/  # QuickJS 沙箱 + JCM 兼容 API + C++ 原生模块宿主
└── mrpc-app/     # egui + wgpu 桌面应用
assets/i18n/      # 7 种界面语言
examples/         # C++ 脚本示例模板（由应用生成）
```

## 兼容性与致谢

- 字段名与行为对齐 MTR 4.0.x 序列化器（camelCase 键、全字段写出）；`mtr_custom_resources.json` 的 `vehicleScripts` 等扩展按晏阳技术组修改版 JCM（BugQuantumCore/Joban-Client-Mod-Yanyang）解析。
- 本项目为独立实现：格式知识来自上述开源项目的阅读，**不包含**任何来自 MTR/TSC/JCM 的代码或素材；许可为 MIT（见 [LICENSE](LICENSE)）。
- 原版“资源包创建器”为 MTR 游戏内的网页界面（Railway Dashboard 启动）；本工具将其理念带到桌面端，并新增 3D 预览、门动画时间轴、VVVF 试听与列车脚本功能。

## 已知边界（v0.2）

- 音频试听需 `--features audio`（Linux 需 ALSA 头文件）；无音频时仍有波形/频谱与 WAV 导出。
- BVE 真实音源回放目前仅支持 WAV 循环（BVE 音源集里的 OGG 需先转 WAV）；`[SoundList]` 未列出的电机声道保持静音。
- MQO/MQOZ 与 glTF 导入已支持常见子集：MQO 的 `chunk`/镜像、glTF 的骨骼/形变/动画/稀疏访问器会被忽略；`tex()` 引用的贴图从模型同目录解析；`natives/<platform>` 多平台映射按 JCM 规则解析与打包。
- C++ 脚本预览使用本工具的预览 ABI（abi=1）；进入游戏需按 JCM 原生 SDK 重新编译。

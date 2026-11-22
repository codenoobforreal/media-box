# stream

index 媒体文件内的流索引号，从0开始计数
codec_name 编码器简称
codec_long_name 编码器完整标准名称
profile 编码档次/配置集：定义编码可用的算法集合，直接影响压缩效率与解码复杂度
codec_type 媒体流类型，video 代表视频流，audio 代表音频流，另有字幕流 subtitles、数据流 data 等其他类型
codec_tag_string 编码器四字符标识(FourCC)，属于封装层编码标识
codec_tag codec_tag_string 对应的十六进制数值
mime_codec_string 遵循RFC 6381规范的MIME编码参数字符串，用于网页、HTTP场景下精准描述编码参数
coded_width 编码分辨率：编码器实际处理帧的原始宽高
has_b_frames GOP图像组中，两个非B帧之间允许连续出现的最大双向预测B帧数量
sample_aspect_ratio 采样宽高比，单个像素自身的宽高比例
display_aspect_ratio 显示宽高比，最终播放画面整体的宽高比例，计算公式：DAR = (画面宽度 × 像素宽) : (画面高度 × 像素高)
pix_fmt 像素格式，定义视频帧的色彩编码、采样方式与内存存储排布
level 编码等级，限定该编码配置支持的最大参数（分辨率、帧率、码率等）；实际等级为此数值除以10
chroma_location 色度分量相对亮度分量的采样坐标位置
field_order 场序，定义视频的扫描方式
is_avc 是否采用AVC封装格式
nal_length_size AVC封装中，每个NAL单元前置长度字段占用的字节数
id 流在封装容器内的唯一标识，即容器层分配的流编号；视频流ID通常为0x1，音频流ID为0x2
r_frame_rate 标称/实时帧率：流预设帧率，使用分数存储保证精度
avg_frame_rate 实际平均帧率，总帧数除以总时长计算得出，反映真实播放帧率
time_base 时间基，当前流所有时间戳的最小计量单位；一个时间戳单位对应固定秒数
start_pts 流首帧的显示时间戳(PTS)，单位为 time_base
duration_ts 流总时长对应的总时间戳数值，单位为 time_base
duration 总播放时长，等于 duration_ts × time_base
bit_rate 流码率，每秒传输/存储的数据量，单位比特每秒(bps)
bits_per_raw_sample 原始采样位深，每个色彩分量采样占用的比特位数
nb_frames 流总帧数
extradata_size 附加私有数据大小，存储解码器初始化所需的专属配置参数，单位字节
sample_fmt 音频采样格式：描述单个采样点的数据类型与存储方式
sample_rate 音频采样率，每秒对声音信号的采样次数
channels 音频声道数量
channel_layout 声道布局，定义声道数量与各声道空间位置
bits_per_sample 此字段仅对无压缩PCM音频有效；AAC等压缩格式该值固定为0，不代表实际采样位深
initial_padding 编码器初始化预留采样数

# format

filename 媒体文件完整路径与文件名
nb_streams 文件包含的媒体流总数量
nb_programs 文件包含的节目数量；节目概念主要用于MPEG-TS直播/广播传输流，可将多路音视频流封装至同一节目；MP4等常规本地文件无节目结构，该值为0
nb_stream_groups 文件包含的流分组数量；流分组多用于多视角视频、多音轨等特殊场景；标准单版本视频文件该值为0
name FFmpeg解封装器简称；逗号分隔列表代表该解封装器支持的全部封装格式
long_name 封装格式完整官方名称
start_time 整个文件的起始时间，取所有流中最早的起始时间
duration 文件总播放时长，取所有流中最长时长作为封装容器总时长
size 媒体文件总大小，单位字节
bit_rate 文件整体平均码率，文件总大小除以总时长计算，包含音视频数据与封装包头开销，单位bps
probe_score 格式探测置信分，FFmpeg用于判定识别出的文件格式可靠程度的指标

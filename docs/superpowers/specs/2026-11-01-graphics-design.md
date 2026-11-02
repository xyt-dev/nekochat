# 通用图形输出设计

日期：2026-11-01。用户已同意「Rust core 通用协议 + 前端共享渲染器 + 角色可选资源包」，要求图形可灵活定制、跟随主题配色、本地交互流畅。

## 范围与选择

采用声明式 `neko-ui` JSON 围栏和静态 `svg` 围栏；原生 SolidJS/SVG 渲染，不引入 DSH 宿主或 React。包含 text/stat、row/col/grid/card、chart、plot、diagram、svg。包内只读 SKILL 提供精简规范和示例；不执行模型生成的 JS，不增加模型动作事件循环、地图、3D 或外部图床。

图形作为普通 assistant Markdown 内容进入已有 SQLite 消息记录；core 提供通道无关的解析校验，WASM 在 Worker 内复用它。历史重新渲染不依赖当前角色是否仍勾选教学包。不增加数据库迁移或独立图形存储。

## 固定接口

新增 `nekochat_core::graphics`：
- `validate_graphic_json(source: &str) -> Result<String, String>`：严格验证并返回规范 JSON。
- `validate_complete_graphic_json(source: &str) -> Result<String, String>`：完成块先按严格路径验证；失败时仅允许补齐匹配且未闭合的对象/数组结束符，再执行全部原有校验。未闭合字符串、缺失字段、非法语法与危险内容仍拒绝。
- `sanitize_graphic_svg(source: &str) -> Result<String, String>`：验证独立 SVG 并返回安全 SVG。
- `sample_graphic_plot(source: &str, values: &str) -> Result<String, String>`：source 为一个 plot 节点，values 为参数名到数值的对象；返回采样结果 JSON。

mdwasm 导出 `validate_graphic(source, complete)`、`sanitize_graphic_svg`、`sample_graphic_plot`；`complete` 仅在围栏关闭或消息结束时允许补齐容器结束符，流式未完成块仍严格解析。成功返回 JSON/SVG 字符串，错误抛出可读字符串。前端只使用已经通过 core/WASM 校验的规范。

### JSON v1

根：`{version:1,title?:string,gap?:number,items:GraphicNode[]}`。

共用节点字段：`id?:string,title?:string,span?:number`；id 在一个图形内唯一。除 choices 外各节点可声明 `palette?:boolean`，只有明确 true 时显示该节点独立调色控件，省略/false 均不显示，拒绝其他类型；不继承父节点开关，不影响其他组件。颜色可使用 `accent,text,muted,surface,border,red,orange,yellow,green,teal,blue,purple,pink` 或 `#RGB/#RRGGBB/#RRGGBBAA`，默认由当前 --nk-* 主题变量决定。

节点：
- text: `{type:"text",content:string,size?:"body"|"h2"|"h3"|"muted",color?:Color}`。
- stat: `{type:"stat",label:string,value:string|number,delta?:string,color?:Color}`。
- choices: `{type:"choices",options:[{label:string,prompt:string}]}`。1–6 项，label 为 1–80 字符，prompt 为 1–500 字符；均须包含非空白内容，去除两端空白后不能重复，不接受链接或执行字段。Web 点击只在当前对话输入框追加完整问题，保留草稿及附件，由用户编辑后正常发送。
- row/col/grid/card: `{type,items:GraphicNode[],gap?:number,cols?:number,color?:Color}`；cols 仅 grid 使用，1–4；card 可使用 title。
- chart: `{type:"chart",kind:"line"|"bar"|"area"|"donut",labels:string[],series:[{label:string,values:number[],color?:Color}],height?:number}`。每条 series 长度等于 labels；donut 仅一条非负 series 且总和大于零；其他图表支持负数。
- plot: `{type:"plot",xMin:number,xMax:number,yMin?:number,yMax?:number,height?:number,params?:[{name:string,label?:string,value:number,min:number,max:number,step?:number}],series:[{label?:string,expr:string,color?:Color}]}`。参数名合法且不覆盖 x/pi/e/tau，值在范围内。表达式仅算术、变量、pi/e/tau 和 sin/cos/tan/asin/acos/atan/sqrt/abs/exp/log/ln/floor/ceil/round/min/max/pow，不使用 eval。采样固定 512 个区间（513 点，包含两端）；非有限函数值输出 null 以断开路径。结果 `{xMin,xMax,yMin,yMax,series:[{label,points:[[x,y|null],...]}]}`。
- diagram: `{type:"diagram",width?:number,height?:number,color?:Color,nodes:[{id:string,label:string,x:number,y:number,w?:number,h?:number,description?:string,color?:Color}],edges:[{from:string,to:string,label?:string}]}`。diagram.color 是未单独配色节点的默认色；节点 color 优先，均省略时使用主题强调色。坐标及尺寸有限，边只能引用存在节点，节点 id 唯一。
- svg: `{type:"svg",code:string,height?:number}`，code 通过相同 SVG 校验。

限制：JSON 128 KiB、最多 128 节点、嵌套最多 8 层、普通文本单项 4000 字符、chart 最多 8 系列/每系列 2048 点、plot 最多 8 系列/8 参数/表达式 256 字符、diagram 最多 32 节点/64 边、SVG 64 KiB/1024 元素。高度 120–800，gap 0–32，span 1–4。拒绝未知字段、未知节点、非有限数值、无效范围及危险颜色表达式。

SVG 使用浏览器原生图片模式绘制，支持基本形状、完整路径/变换、文字/tspan/textPath、线性/径向渐变、clipPath/mask/pattern/marker、filter 及滤镜原语、defs/symbol/use、本地 href/xlink、内嵌位图与 SVG 图片、内嵌字体、安全样式属性/样式表/CSS 关键帧、SMIL 声明式动画，以及图片模式可绘制的 foreignObject/XHTML 排版。Rust/WASM 统一校验 XML、CSS 令牌和资源引用并重建命名空间，不用有限的颜色/绘图属性子集代替原生 SVG 语法。嵌套 data SVG 递归通过同一校验与主题准备，并共享体积/元素预算；保留深度和引用循环/展开量限制。拒绝脚本、事件处理、外部网络引用、可执行 URL、DTD/ENTITY 与用户提供的处理指令。最终绘图始终作为隔离 Blob 图片呈现；脚本、链接导航及其他文档交互不属于图片上下文能力。各浏览器对 SVG 绘图特性的具体实现仍以原生引擎为准。

无障碍 `aria-labelledby` / `aria-describedby` 仅接受本 SVG 内已定义的 ID 引用列表，拒绝 URL、片段前缀和不存在的目标，保留静态 title/desc 语义。

主题变量通过生成的可信 data CSS 处理指令提供低优先级默认值，不往根元素添加样式子节点，保留源 SVG 的结构选择器、CSS 层叠、局部变量、currentColor 继承与渐变关系；兼容 `--nk-muted` 到 `--nk-text-muted` 的回退，宿主显式提供别名时尊重其值。只有 SMIL 参数实际含变量时，才把已校验文档加载到临时 `sandbox="allow-same-origin"`、禁用脚本的隔离 iframe，用原生动画 targetElement 和 getComputedStyle 读取目标元素的变量，再立即移除文档并回收 URL；不把 SVG 插入宿主元素树。替换与卸载通过 AbortSignal 取消过期准备。主题变化时重建并回收旧 Blob，图形无障碍名称继承源 SVG 的 aria-label/title。

实现依据：[SVG 2 secure animated mode](https://www.w3.org/TR/SVG2/conform.html#secure-animated-mode) 定义图片模式的声明式动画、无脚本/外部资源边界；XML 由 roxmltree 解析，CSS 令牌复用 Servo [cssparser](https://docs.rs/cssparser/)。对照了 [DOMPurify 的 SVG 配置及输出上下文警告](https://github.com/cure53/DOMPurify#can-i-configure-dompurify)：它面向可插入 DOM 的清理，不能单独提供本项目跨 Rust/WASM 的资源预算与无外链规则；本项目不引入第二套 DOM 清理/绘图规则，保留原生图片隔离，主题处理仅在已校验内容内进行。

## 前端边界与性能

`web/src/graphics/GraphicBlock.tsx` 默认导出组件：props `{id:string,spec:GraphicSpec,source:string,onQuestion?:(prompt:string)=>void,questionsDisabled?:boolean}`。`web/src/graphics/types.ts` 导出共享 TS 视图类型；`state.ts` 导出 `resetGraphicsRuntime()` 给账号退出/切换路径使用。渲染器按需载入。choices 复用 Markdown/图形校验路径，只在完整成功回复且当前对话未生成/编辑时允许选择；选择事件经消息组件追加输入草稿并聚焦，不能直接发起模型请求。只包含 choices 的根使用紧凑选项样式。

Worker Markdown 响应从 html 字符串改为类型块：`{kind:"html",html}`、`{kind:"graphic",spec,source}`、`{kind:"graphic-pending"}`、`{kind:"graphic-error",error,source}`。有效的完整 JSON 可以在围栏关闭前显示；未完成 JSON 保持占位，不逐 token 展示错误或重复挂载。围栏关闭或消息结束后，允许仅补齐可确定的对象/数组结束符再严格验证；原始源码不改写，其余错误明确显示。

原有 Index 块节点保持稳定，图形内部使用节点 id/路径保留组件；普通 Markdown 不受影响。plot 采样使用独立 Worker 调 core/WASM，同一交互只保留最新参数，合并帧更新并丢弃过期结果；虚拟滚动卸载时移除监听和待处理任务。图形交互状态仅保存参数、配色和视图选择，可用受限 sessionStorage 配合有界内存缓存保存，不能保存消息正文/凭证；退出时清理缓存和 Worker。图片 URL、观察器和事件监听均按生命周期释放。

所有默认颜色使用现有 --nk-* 语义变量；原生图表直接继承 CSS 变量，裸 SVG 在主题变化时重建隔离图片。默认移除所有调色入口；只有节点明确 `palette:true` 才显示所属组件的独立 popover，局部「跟随主题」也只恢复这一组件。容器调色仅修改自身背景，不修改后代颜色或为后代启用调色；整图入口、跨组件重置与全局主题色覆盖已移除。图表切换、函数滑块和源规范查看/复制继续本地执行。

纯 choices 保持紧凑样式。其他图形标题仅显示标题文本；底部以小字「源规范」折叠查看原文，展开的源码框内右上角放复制按钮与反馈，代码独立滚动且不被控件遮挡。叙事 SVG 提示以鲸鱼与星座示例展示留白、上下呼应、渐变和主题颜色，实际回答根据上下文重新构图。

## 资源包

内置 `resource-packs/genui/{pack.json,SKILL.md,stickers.json}`，stickers 为 []。空素材的通用指令包不套用表情身份/频率/非商业说明；表情包仍保留自己的规则和署名。角色编辑器把 0 张素材的包标成指令包。现有包和默认角色不自动修改。

## 验收

真实页面覆盖：混合 Markdown/图形流式回复；有效不完整围栏；非法 JSON/SVG 提示；桌面/390px；八套主题（含深浅）和自定义变量；调色、切图、拖动参数；虚拟滚动离开再返回及刷新恢复；用户退出后状态隔离；主题变化时 SVG 配色同步；观察器/Worker/URL 回收。测量连续调参的 rAF 帧间隔、长任务和操作延迟，记录条件和实测值，不承诺所有硬件绝不卡。

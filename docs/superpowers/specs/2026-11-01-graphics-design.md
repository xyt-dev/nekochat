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

共用节点字段：`id?:string,title?:string,span?:number`；id 在一个图形内唯一。颜色可使用 `accent,text,muted,surface,border,red,orange,yellow,green,teal,blue,purple,pink` 或 `#RGB/#RRGGBB/#RRGGBBAA`。默认由当前 --nk-* 主题变量决定。

节点：
- text: `{type:"text",content:string,size?:"body"|"h2"|"h3"|"muted",color?:Color}`。
- stat: `{type:"stat",label:string,value:string|number,delta?:string,color?:Color}`。
- choices: `{type:"choices",options:[{label:string,prompt:string}]}`。1–6 项，label 为 1–80 字符，prompt 为 1–500 字符；均须包含非空白内容，去除两端空白后不能重复，不接受链接或执行字段。Web 点击只在当前对话输入框追加完整问题，保留草稿及附件，由用户编辑后正常发送。
- row/col/grid/card: `{type,items:GraphicNode[],gap?:number,cols?:number,color?:Color}`；cols 仅 grid 使用，1–4；card 可使用 title。
- chart: `{type:"chart",kind:"line"|"bar"|"area"|"donut",labels:string[],series:[{label:string,values:number[],color?:Color}],height?:number}`。每条 series 长度等于 labels；donut 仅一条非负 series 且总和大于零；其他图表支持负数。
- plot: `{type:"plot",xMin:number,xMax:number,yMin?:number,yMax?:number,height?:number,params?:[{name:string,label?:string,value:number,min:number,max:number,step?:number}],series:[{label?:string,expr:string,color?:Color}]}`。参数名合法且不覆盖 x/pi/e/tau，值在范围内。表达式仅算术、变量、pi/e/tau 和 sin/cos/tan/asin/acos/atan/sqrt/abs/exp/log/ln/floor/ceil/round/min/max/pow，不使用 eval。采样固定 512 个区间（513 点，包含两端）；非有限函数值输出 null 以断开路径。结果 `{xMin,xMax,yMin,yMax,series:[{label,points:[[x,y|null],...]}]}`。
- diagram: `{type:"diagram",width?:number,height?:number,nodes:[{id:string,label:string,x:number,y:number,w?:number,h?:number,description?:string,color?:Color}],edges:[{from:string,to:string,label?:string}]}`。坐标及尺寸有限，边只能引用存在节点，节点 id 唯一。
- svg: `{type:"svg",code:string,height?:number}`，code 通过相同 SVG 校验。

限制：JSON 128 KiB、最多 128 节点、嵌套最多 8 层、普通文本单项 4000 字符、chart 最多 8 系列/每系列 2048 点、plot 最多 8 系列/8 参数/表达式 256 字符、diagram 最多 32 节点/64 边、SVG 64 KiB/1024 元素。高度 120–800，gap 0–32，span 1–4。拒绝未知字段、未知节点、非有限数值、无效范围及危险颜色表达式。

SVG 仅允许静态几何/文字/渐变/裁切元素与有限属性；禁止脚本、事件、foreignObject、动画、外部资源、外部 href、DOCTYPE/ENTITY 和 style 元素。支持 fill/stroke 的 currentColor 与 var(--nk-*) 主题颜色，以及安全的片段渐变引用 url(#id)。通过隔离图片模式绘制；主题变化时替换主题值并回收旧 Blob URL，不把原始模型 SVG 写入宿主 DOM。

## 前端边界与性能

`web/src/graphics/GraphicBlock.tsx` 默认导出组件：props `{id:string,spec:GraphicSpec,source:string,onQuestion?:(prompt:string)=>void,questionsDisabled?:boolean}`。`web/src/graphics/types.ts` 导出共享 TS 视图类型；`state.ts` 导出 `resetGraphicsRuntime()` 给账号退出/切换路径使用。渲染器按需载入。choices 复用 Markdown/图形校验路径，只在完整成功回复且当前对话未生成/编辑时允许选择；选择事件经消息组件追加输入草稿并聚焦，不能直接发起模型请求。只包含 choices 的根使用紧凑选项样式。

Worker Markdown 响应从 html 字符串改为类型块：`{kind:"html",html}`、`{kind:"graphic",spec,source}`、`{kind:"graphic-pending"}`、`{kind:"graphic-error",error,source}`。有效的完整 JSON 可以在围栏关闭前显示；未完成 JSON 保持占位，不逐 token 展示错误或重复挂载。围栏关闭或消息结束后，允许仅补齐可确定的对象/数组结束符再严格验证；原始源码不改写，其余错误明确显示。

原有 Index 块节点保持稳定，图形内部使用节点 id/路径保留组件；普通 Markdown 不受影响。plot 采样使用独立 Worker 调 core/WASM，同一交互只保留最新参数，合并帧更新并丢弃过期结果；虚拟滚动卸载时移除监听和待处理任务。图形交互状态仅保存参数、配色和视图选择，可用受限 sessionStorage 配合有界内存缓存保存，不能保存消息正文/凭证；退出时清理缓存和 Worker。图片 URL、观察器和事件监听均按生命周期释放。

所有默认颜色使用现有 --nk-* 语义变量；显式配色可通过局部设置恢复主题默认。原生 SVG 图表直接继承 CSS 变量，裸 SVG 在配色变化时重建隔离图片。提供本地参数滑块、图表视图选择、配色调整和源规范查看/复制；不产生模型请求。配色入口显示「配色」，使用浏览器顶层 popover 小浮层，避开布局裁切并限制在可见视口；支持内部滚动、外部点击关闭、Escape 和关闭按钮。色块及完整 HEX 输入即时应用，不使用系统选色框或确认步骤；监听器仅在浮层打开时注册并在关闭/卸载时回收。

标题右侧仅保留「配色」入口；「恢复主题配色」放入此浮层，包含没有布局节点的图表或指标。图形底部以小字「源规范」折叠入口查看原文，复制按钮和操作状态仅在展开区域显示；纯 choices 保持紧凑样式。

## 资源包

内置 `resource-packs/genui/{pack.json,SKILL.md,stickers.json}`，stickers 为 []。空素材的通用指令包不套用表情身份/频率/非商业说明；表情包仍保留自己的规则和署名。角色编辑器把 0 张素材的包标成指令包。现有包和默认角色不自动修改。

## 验收

真实页面覆盖：混合 Markdown/图形流式回复；有效不完整围栏；非法 JSON/SVG 提示；桌面/390px；八套主题（含深浅）和自定义变量；调色、切图、拖动参数；虚拟滚动离开再返回及刷新恢复；用户退出后状态隔离；主题变化时 SVG 配色同步；观察器/Worker/URL 回收。测量连续调参的 rAF 帧间隔、长任务和操作延迟，记录条件和实测值，不承诺所有硬件绝不卡。

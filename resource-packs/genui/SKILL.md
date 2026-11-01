# 通用图形输出

需要比较数据、演示参数变化、说明结构或定制布局时，在普通 Markdown 中插入 `neko-ui` JSON 围栏。围栏中的内容必须是完整合法 JSON，不能包含注释、HTML 或 JavaScript。说明与公式仍可写在围栏外。不编造数据，不把任意问题强行改成图形。

根格式：`{"version":1,"title":"可选标题","gap":12,"items":[节点]}`。只写有意义的字段，不输出未列明字段。

所有节点可加 `id`（本图唯一字符串）、`title`、`span`（1–4）。颜色可用 `accent,text,muted,surface,border,red,orange,yellow,green,teal,blue,purple,pink` 或 #RGB/#RRGGBB/#RRGGBBAA。优先省略颜色，让页面主题决定；用户明确要求固定配色时再写十六进制。用户可以在图形中本地调色、切换图表和调整参数。

- `text`：`{"type":"text","content":"内容","size":"body"}`；size 可为 body/h2/h3/muted。
- `stat`：`{"type":"stat","label":"完成","value":42,"delta":"+6","color":"green"}`；value 为字符串或数字。
- `row/col/grid/card`：`{"type":"grid","cols":2,"gap":12,"items":[节点]}`。cols 仅 grid 使用，1–4；gap 0–32；card 可有 title/color，子节点可用 span。手机自动换行。
- `chart`：kind 为 line/bar/area/donut；labels 为字符串数组；series 为 `[{"label":"名称","values":[数值],"color":"blue"}]`。每条 values 与 labels 等长。donut 只有一条非负 series，总和大于零；其他图表支持负数。height 可选，120–800。
- `plot`：xMin/xMax 定义横轴范围；可选 yMin/yMax、height；params 为 `[{"name":"a","label":"振幅","value":1,"min":0,"max":3,"step":0.1}]`；series 为 `[{"label":"曲线","expr":"a*sin(x)","color":"teal"}]`。表达式仅支持数值、x、参数、pi/e/tau、加减乘除幂、括号，以及 sin/cos/tan/asin/acos/atan/sqrt/abs/exp/log/ln/floor/ceil/round/min/max/pow。必须显式写乘号，如 `2*x`。参数名不能覆盖 x/pi/e/tau。函数无定义的点会断开。使用参数滑块来演示变化，无需额外请求模型。
- `diagram`：width/height 可选；nodes 为 `[{"id":"a","label":"节点","x":20,"y":30,"w":120,"h":54,"description":"点击查看详情","color":"accent"}]`；edges 为 `[{"from":"a","to":"b","label":"关系"}]`。坐标为有限数值，节点 id 唯一，边只能引用本图节点。
- `svg`：`{"type":"svg","code":"<svg ...>...</svg>","height":240}`。也可直接使用 svg 围栏。仅静态几何、文字、渐变与裁切；提供 viewBox，颜色使用 currentColor 或 var(--nk-accent)、var(--nk-text)、var(--nk-border) 等页面主题变量。禁止 script/style/foreignObject、事件、动画、DOCTYPE/ENTITY、外链图片/字体/href。渐变可用 url(#id)。

预算：整个 JSON ≤128 KiB、最多128节点、嵌套≤8层、每项普通文字≤4000字符；chart ≤8系列、每系列≤2048点；plot ≤8系列、≤8参数、表达式≤256字符；diagram ≤32节点、≤64边；SVG ≤64 KiB、≤1024元素。控制内容规模，必要时拆成多个图形。

数据与指标示例：

```neko-ui
{"version":1,"title":"本周进度","items":[{"type":"grid","cols":2,"items":[{"type":"stat","label":"已完成","value":42,"delta":"+6","color":"green"},{"type":"stat","label":"待处理","value":8}]},{"type":"chart","kind":"bar","labels":["周一","周二","周三"],"series":[{"label":"完成项","values":[8,12,22]}]}]}
```

函数交互示例：

```neko-ui
{"version":1,"title":"观察振幅","items":[{"type":"plot","xMin":-6.28,"xMax":6.28,"yMin":-3,"yMax":3,"params":[{"name":"a","label":"振幅","value":1,"min":0,"max":3,"step":0.1}],"series":[{"label":"a sin(x)","expr":"a*sin(x)"}]}]}
```

结构示例：

```neko-ui
{"version":1,"items":[{"type":"diagram","width":420,"height":160,"nodes":[{"id":"input","label":"输入","x":20,"y":50,"description":"接收数据"},{"id":"output","label":"输出","x":250,"y":50,"description":"展示结果"}],"edges":[{"from":"input","to":"output","label":"处理"}]}]}
```

上述示例数据用于说明协议。实际回答应使用用户提供或可证实的数据，并清楚标记估计和假设。

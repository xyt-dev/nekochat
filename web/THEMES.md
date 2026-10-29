# 主题

聊天页的全部颜色都来自 CSS 变量。一个主题就是一个 CSS 文件，只做一件事：在 `:root[data-theme="<id>"]` 上给出下面这组变量。

## 内置主题

| id | 名称 | 分组 |
|---|---|---|
| `catppuccin-latte` | Latte | Catppuccin |
| `catppuccin-frappe` | Frappé | Catppuccin |
| `catppuccin-macchiato` | Macchiato | Catppuccin |
| `catppuccin-mocha` | Mocha | Catppuccin |
| `gruvbox-dark` | Dark | Gruvbox |
| `gruvbox-light` | Light | Gruvbox |
| `claude` | Light | Claude |
| `claude-dark` | Dark | Claude |

文件在 `web/src/themes/`。新访客默认 Claude Dark；已有选择保留在 localStorage（`nk.theme`，自定义主题另记 `nk.themeUrl`），不因明暗系统设置或升级覆盖个人选择。

## 变量清单

自定义主题必须给出**全部**变量，缺了的会让对应元素失去颜色。

| 变量 | 用途 | Catppuccin 对应 |
|---|---|---|
| `color-scheme` | `light` 或 `dark`（滚动条、表单控件的配色） | — |
| `--nk-bg` | 页面与对话区底色 | base |
| `--nk-bg-side` | 侧栏底色；没给 `--nk-bg-raised` 时也用于输入框与弹出菜单 | mantle |
| `--nk-bg-raised` | 可选：输入框、弹出菜单、口令输入框底色 | — |
| `--nk-bg-deep` | 代码块底色 | crust |
| `--nk-surface` | 用户气泡、悬停与选中底色、行内代码 | surface0 |
| `--nk-surface-hover` | 更强的高亮（对话树当前位置、提示条） | surface1 |
| `--nk-border` | 边框、分隔线 | surface1 |
| `--nk-text` | 正文 | text |
| `--nk-text-muted` | 次要文字（思考全文、侧栏标题） | subtext0 |
| `--nk-text-faint` | 最弱的文字（模型名、debug 行标签、占位符） | overlay0 |
| `--nk-accent` | 主色（发送按钮、焦点边框、选中标记） | mauve |
| `--nk-on-accent` | 主色上面的文字 | crust / base |
| `--nk-red` | 错误、删除按钮、停止按钮 | red |
| `--nk-orange` | 「已中断」标记、代码里的数字 | peach |
| `--nk-yellow` | debug 行「输出」、代码里的类型 | yellow |
| `--nk-green` | debug 行「首字」「速率」、代码里的字符串 | green |
| `--nk-teal` | debug 行「缓存输入」、代码里的属性 | teal |
| `--nk-blue` | debug 行「无缓存输入」、链接、代码里的函数名 | blue |
| `--nk-purple` | debug 行「思考」、思考强度、代码里的关键字 | mauve |
| `--nk-pink` | 代码里的元信息 | pink |

主题菜单里每项的三个色点依次是 `--nk-bg`、`--nk-accent`、`--nk-text`。

共享布局另外提供 `--nk-sidebar-width`、`--nk-content-width` 和 `--nk-radius-sm` / `--nk-radius-md` / `--nk-radius-lg`，统一侧栏、内容列和控件圆角。这些是可选的几何覆盖，不属于自定义主题必填的调色变量；颜色仍只来自上表。触屏输入控件保持至少 16px，不通过视口缩放限制适配主题。

## 自定义主题

1. 在服务端数据目录下建 `themes/`（例如 `./data/themes/`）。
2. 放入 `<id>.css`，文件名只能用字母、数字、`_`、`-`；文件名去掉 `.css` 就是主题的 id 和显示名。
3. 选择器写 `:root[data-theme="<id>"]`，id 必须与文件名一致。
4. 刷新页面，主题出现在切换菜单的「自定义」分组里，不用重启服务。

例：`data/themes/sakura.css`

```css
:root[data-theme="sakura"] {
  color-scheme: light;
  --nk-bg: #fff5f7;
  --nk-bg-side: #fbe9ee;
  --nk-bg-deep: #f6dde4;
  --nk-surface: #f8e1e8;
  --nk-surface-hover: #f2d0da;
  --nk-border: #ecc5d0;
  --nk-text: #3a2a30;
  --nk-text-muted: #6b5560;
  --nk-text-faint: #9a8790;
  --nk-accent: #e75480;
  --nk-on-accent: #ffffff;
  --nk-red: #c8364f;
  --nk-orange: #d9733b;
  --nk-yellow: #b0862a;
  --nk-green: #4f8a4a;
  --nk-teal: #2f8a80;
  --nk-blue: #3f6fb5;
  --nk-purple: #9356b8;
  --nk-pink: #c4528f;
}
```

别和内置主题重名（例如不要叫 `claude.css` 或 `claude-dark.css`），否则两份规则会叠在一起。

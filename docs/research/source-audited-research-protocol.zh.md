# Source-Audited Research Protocol 中文版

日期：2026-07-03

目的：防止竞品调研再次出现“凭印象判断”“把未知写成没有”“把相似 UX 写成同构架构”“把我们的目标能力误写成已验证事实”的问题。

## 1. 可用技能和适用边界

| 技能 / 流程 | 适合做什么 | 不适合做什么 |
| --- | --- | --- |
| `ask-matt` | 选择应该走 `/grill-with-docs`、`/to-prd`、`/to-issues`、`/implement` 还是 `/triage`。 | 不能替代调研证据。它是流程路由器，不是事实来源。 |
| `grill-with-docs` | 在有代码库时先追问目标、边界、术语，并把共识沉淀进 docs / ADR / glossary。 | 不能证明外部产品能力。它解决“我们要什么”，不是“竞品实际有什么”。 |
| `validate-data` | 审查表格、对比、结论、方法论、 caveat 是否足够支撑决策。适合审查 benchmark/report。 | 不是自动抓取器。它要求已有 artifact 或 claim ledger。 |
| `openai-docs` | 调研 Codex / OpenAI 相关能力时，优先走官方 OpenAI/Codex 文档路线。 | 不适用于 Claude、Cursor、Grok、OpenCode 等非 OpenAI 产品。 |
| `github:github` / `gh` | 查看本项目 issue / PR / review comment / 仓库状态。 | 不是外部产品事实来源，除非调研的是对应 GitHub 官方仓库源码。 |
| `to-prd` | 在调研和对齐完成后，把已确认路线写成 PRD。 | 不能拿不完整调研直接生成 PRD。 |
| `to-issues` | 把已批准 PRD 拆成 agent-ready issues。 | 不能在 source audit gate 未通过时拆实现 issue。 |

结论：这里没有一个“万能调研技能”能保证不犯错。正确做法是组合流程：

```text
ask-matt
-> grill-with-docs 对齐问题和术语
-> source-audited research
-> validate-data 审查调研表格和结论
-> owner 对齐
-> to-prd
-> to-issues
-> implement
```

## 2. Source-Audited Research 硬规则

### 2.1 未找到证据不等于没有

禁止写法：

```text
Cursor 没有 X
Codex 不支持 Y
OpenCode 没有 Z
```

除非官方文档、官方源码、官方 release note 明确写了“不支持 / deprecated / removed”。

正确写法：

```text
公开资料未确认 X。
当前抓取到的官方页面未证明 Y。
该能力需要进一步查官方源码或实测。
```

### 2.2 每个能力判断必须落到 claim ledger

每个产品每个能力格子都要能追到一条 claim：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `product` | 是 | 产品名。 |
| `capability` | 是 | 能力维度，例如 MCP、skills、worktree、background、commit/PR。 |
| `claim` | 是 | 最小事实判断。 |
| `evidence_url` | 是 | 官方文档、官方源码、官方博客、release note 或 GitHub 源码链接。 |
| `evidence_type` | 是 | official docs / official source / official blog / community / local verification。 |
| `confidence` | 是 | A/B/C/D。 |
| `checked_at` | 是 | 检查日期。 |
| `impact_on_harness` | 是 | 这个事实如何影响本项目路线。 |
| `unknowns` | 是 | 仍未确认的点。 |

### 2.3 信心等级

| 等级 | 定义 | 能否进 PRD / issue |
| --- | --- | --- |
| A | 官方文档或官方源码直接确认。 | 可以进 PRD / issue。 |
| B | 官方索引、官方搜索片段、官方博客、相邻页面确认方向，但细节未完全抽取。 | 可以进 PRD，拆 issue 前要补 A 级或明确 caveat。 |
| C | 社区文章、论坛、第三方教程、个人经验。 | 只能当线索，不可作为实现边界。 |
| D | 未找到证据。 | 只能写未知，不能推动实现决策。 |

### 2.4 术语必须先归一

以下词不能混用：

| 术语 | 含义 |
| --- | --- |
| workflow | 流程、自动化、任务步骤。 |
| worktree | Git 隔离 checkout。 |
| workspace | 工作目录或云端/本地工作环境，未必是 Git worktree。 |
| checkpoint | UI/产品层回滚点或变更快照，未必是 EventLog。 |
| commit handoff | 本项目术语：task-scoped approval 后，由 harness 从 exact diff-producing repo_path 做 durable git commit。 |
| approval | 用户/策略批准动作，未必等同于本项目 Approval State Machine。 |
| tool runtime | 执行工具的边界；不等于“工具列表”。 |
| plugin / skill / hook / MCP | 都是扩展面，但信任模型和执行时机不同。 |

## 3. 调研交付物必须包含什么

一个合格 benchmark 至少包含：

1. 产品清单。
2. 能力维度定义。
3. 总体能力矩阵。
4. 每个产品的资料来源表。
5. 每个产品的能力面图。
6. 每个产品的架构图。
7. 每个产品的执行流程图。
8. 每个争议能力的 claim ledger。
9. 未确认项列表。
10. 对本项目的“可借鉴 / 不可照搬 / 影响路线”。
11. owner decision gate：哪些结论已批准，哪些只是候选。

## 4. PRD / Issues 前的 Gate

在 `/to-prd` 前，必须满足：

- 调研文档存在。
- 每个高影响结论有 A/B 级来源。
- D 级未知项没有被写成产品缺陷。
- 内部术语已经归一。
- 已明确哪些能力是“成熟产品也有”，哪些是“我们自己的治理边界”。

在 `/to-issues` 前，必须满足：

- PRD 引用 benchmark section。
- 每个 issue 引用具体能力维度和 confidence level。
- 每个 issue 保留 EventLog、Task Lease、Policy Gate、Approval State Machine、Commit Handoff 不可绕过边界。
- Phase B/C 的外部生态能力不能在 Phase A runtime core 未闭环前混入。

## 5. 二次审查流程

每份调研必须做两遍：

### Pass 1: Researcher

产出 facts、sources、能力图、流程图、矩阵。

### Pass 2: Validator

按 `validate-data` 的方式审查：

- 问题是否答对了。
- 来源是否足够新、足够官方。
- 表格每个格子是否能追溯到 evidence。
- 结论有没有越过证据。
- 图是否表达了事实，还是表达了推测。
- caveat 是否靠近对应结论。

验证结果必须给出：

```text
Ready to use / Use with caveats / Needs revision
```

`Needs revision` 时不得进入 `/to-prd` 或 `/to-issues`。

## 6. 本项目的固定防错口径

以后写 coding-agent-harness benchmark 时，默认口径是：

```text
成熟产品大多已经具备 tools、permissions、MCP、skills、plugins、hooks、
subagents、worktrees、background、cloud、memory、review/checkpoint 等能力面。

本项目不应声称“别人没有这些功能”。

本项目要比较的是：
1. 谁拥有 source of truth；
2. tool / extension / worker 是否可治理；
3. 状态是否可 replay；
4. approval 是否 task-scoped；
5. commit/PR/diff 是否绑定真实产出 workspace；
6. 外部 worker 是否只能产 evidence，而不能接管 runtime truth。
```

## 7. 最小执行清单

每次调研前先贴这个清单：

```text
[ ] 我已经定义能力维度，而不是直接填产品印象。
[ ] 每个产品先建 source table。
[ ] 每个能力格子都有 confidence level。
[ ] 未找到证据写 D/未知，不写没有。
[ ] 内部术语和外部术语分开。
[ ] 图里的每条边都有来源或明确标注为推断。
[ ] 结论分为 confirmed / inferred / unknown。
[ ] PRD 前做 validate-data 风格二次审查。
[ ] owner 确认后才 to-prd。
[ ] PRD 确认后才 to-issues。
```

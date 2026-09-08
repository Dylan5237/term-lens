# LLM 常见名词体系化分类 + 术语词汇字典

> **仓库内地位**：本文件是 `ai_terms_latest.csv` 的来源笔记，不是运行时词表。下方「详细文档导航」指向的 `01-*.md` **不在本仓库**。
> **读法建议**：先按「层」建立地图；每层先掌握 **加粗** 的核心词，其它词当作扩展。
> 
> **证据锚点**：as of 2026-01-05，所有术语定义均附来源引用，详见文末参考文献。

---

## 📚 详细文档导航

点击下方链接查看各主题的完整解释、示例与参考文献：

| 章节 | 详细文档 | 术语数 |
|------|----------|--------|
| 1. 输入表示与基础概念 | [01-输入表示与基础概念.md](./01-输入表示与基础概念.md) | 9 |
| 2. 模型结构与规模规律 | [02-模型结构与规模规律.md](./02-模型结构与规模规律.md) | 4 |
| 3. 训练、微调与对齐 | [03-训练微调与对齐.md](./03-训练微调与对齐.md) | 8 |
| 4. 提示工程与推理脚手架 | [04-提示工程与推理脚手架.md](./04-提示工程与推理脚手架.md) | 16 |
| 5. 检索增强与外部知识 | [05-检索增强与外部知识.md](./05-检索增强与外部知识.md) | 4 |
| 6. 推理解码与采样控制 | [06-推理解码与采样控制.md](./06-推理解码与采样控制.md) | 13 |
| 7. 工具调用、结构化输出与智能体 | [07-工具调用结构化输出与智能体系统.md](./07-工具调用结构化输出与智能体系统.md) | 12 |
| 8. 安全、评测与可靠性 | [08-安全评测与可靠性.md](./08-安全评测与可靠性.md) | 6 |

---

## 1. 输入表示与基础概念（Representation & Basics）

| 术语 | 中文 | 定义 | 来源 |
|------|------|------|------|
| **LLM** | Large Language Model | 大型语言模型；以大量文本训练、能生成/理解语言的模型家族统称 | [1] |
| **Model** | 模型 | 一个可被调用来生成输出的具体模型ID/版本（API里通过 `model` 指定） | [3] |
| **Token** | 标记 | 模型处理文本的基本单位，可短至字符或长至单词，取决于语言与上下文 | [6] |
| **Tokenization** | 分词/切分 | 把原始文本切成 token 的过程；常见做法是子词切分 | [8] |
| **BPE** | Byte Pair Encoding | 一种常用子词切分思想（在子词单元构建中使用） | [8] |
| **tiktoken** | - | OpenAI 开源的快速 BPE tokenizer 库 | [7] |
| **Embedding** | 嵌入向量/表征 | 把文本映射到向量空间的表示，便于做相似度检索、聚类等 | [5] |
| **Context window** | 上下文窗口 | 一次推理可利用的输入上下文容量（通常以 token 计）；超出会被截断或需压缩 | [3] ⚠️ |
| **Inference** | 推理/生成 | 给定输入消息，让模型生成输出的过程（API"生成响应"即推理） | [3] |

---

## 2. 模型结构与规模规律（Architecture & Scaling）

| 术语 | 中文 | 定义 | 来源 |
|------|------|------|------|
| **Transformer** | - | 主流LLM的基础架构（"Attention Is All You Need"提出） | [10] |
| **Attention / Self-Attention** | 注意力/自注意力 | Transformer的核心机制，用于在序列内部建立依赖关系 | [10] |
| **Scaling laws** | 规模定律 | 讨论模型/数据/算力规模与性能关系的经验规律 | [12] |
| **Few-shot / In-context learning** | 小样本/上下文学习 | 通过在提示中给少量示例，让模型在不更新参数情况下完成任务（GPT-3论文系统展示） | [11] |

---

## 3. 训练、微调与对齐（Training, Fine-tuning & Alignment）

| 术语 | 中文 | 定义 | 来源 |
|------|------|------|------|
| **Pretraining** | 预训练 | 在大规模通用数据上训练语言建模能力 | ⚠️ |
| **Fine-tuning** | 微调 | 在特定数据/目标上继续训练以改变行为；例如 InstructGPT 用人类反馈进行微调 | [18] |
| **Instruction tuning / SFT** | 指令微调/监督微调 | 用"指令-回答"监督数据让模型更会"听指令" | [18] ⚠️ |
| **RLHF** | Reinforcement Learning from Human Feedback | 用人类偏好信号训练奖励/偏好，再用强化学习或相关方法优化模型行为（InstructGPT路径） | [18] |
| **Reward model** | 奖励模型 | 把"人更喜欢哪个回答"变成可优化的打分信号的模型 | ⚠️ |
| **DPO** | Direct Preference Optimization | 一种直接用偏好数据优化策略的训练方法（不必显式训练奖励模型/跑RL的经典形式） | [19] |
| **LoRA** | Low-Rank Adaptation | 参数高效微调方法，用低秩矩阵注入适配能力，减少训练参数量 | [20] |
| **Checkpoint / Snapshot** | 检查点/快照 | 训练过程中保存的模型参数版本；OpenAI也建议生产上固定到具体"model snapshot"以保持一致性 | [1] |

---

## 4. 提示工程与"推理脚手架"（Prompting & Reasoning Scaffolds）

| 术语 | 中文 | 定义 | 来源 |
|------|------|------|------|
| **Prompt** | 提示词/输入指令 | 你给模型的输入内容（含任务、约束、上下文等）；提示工程就是系统化写它 | [1] |
| **Prompt engineering** | 提示工程 | 写出能稳定得到目标输出的有效指令（"艺术+科学"） | [1] |
| **System message / System prompt** | 系统消息/系统提示 | 用于设定助手行为、边界、格式等的高优先级指令框架 | [24] |
| **Developer message** | 开发者消息 | OpenAI 对推理模型提示建议里提到的更符合"指令链条"的高优先级消息形态（特定模型起强调"developer messages are the new system messages"） | [2] |
| **User message** | 用户消息 | 用户真实输入的请求内容（在"角色消息"体系里区分） | [3] |
| **Message roles** | 消息角色 | 通过不同角色/通道组织指令与对话内容，以影响模型如何遵循指令 | [1][3] |
| **CoT** | Chain-of-Thought prompting | 通过"链式思维"方式引导多步推理的提示范式 | [13] |
| **Self-Consistency** | 自洽性 | 对同一问题采样多条推理路径再投票/汇总，提高CoT推理稳定性 | [14] |
| **ToT** | Tree of Thoughts | 把推理组织成树搜索（扩展/评估/回溯），而不是单一路径 | [15] |
| **ReAct** | Reasoning + Acting | 把"推理"和"行动（如调用工具/检索）"交织在一起的范式 | [17] |
| **Reflection / Reflexion** | 反思式迭代 | 让代理从失败/反馈中生成"语言形式的反思"来改进后续行动 | [21] |
| **Thinking / internal chain-of-thought** | 内部思考 | OpenAI文档提到"推理模型会生成内部chain-of-thought"；同时也强调不一定需要用户显式要求"step by step" | [1][2] |
| **Hallucination** | 幻觉 | 模型生成看似合理但不真实/无依据内容的现象 | ⚠️ |

---

## 5. 检索增强与外部知识（Retrieval & Grounding）

| 术语 | 中文 | 定义 | 来源 |
|------|------|------|------|
| **RAG** | Retrieval-Augmented Generation | 先检索外部文档/证据，再把检索结果作为上下文用于生成答案的框架 | [16] |
| **Embeddings-based search** | 基于嵌入的语义搜索 | 把查询与文档都变成向量，用相似度（如余弦相似度）找最相关内容 | [5] |
| **Cosine similarity** | 余弦相似度 | 衡量向量相似度的常用指标；OpenAI embeddings 示例使用它做检索排序 | [5] |
| **Chunking** | 分块 | 把长文档切成小段以便嵌入与检索 | ⚠️ |

---

## 6. 推理解码与采样控制（Decoding & Generation Control）

| 术语 | 中文 | 定义 | 来源 |
|------|------|------|------|
| **Decoding** | 解码/生成策略 | 把模型给出的分布（logits→概率）转成具体输出token序列的策略总称 | [9] |
| **Logits** | 对数分数 | 采样前的未归一化分数；HF文档在"next_token_logits"语境中讨论其后处理 | [9] |
| **Temperature** | 温度 | 调整下一个token概率分布"尖锐/平滑"的参数 | [9] |
| **Top-k sampling** | - | 只保留概率最高的K个候选token再采样 | [9] |
| **Top-p / Nucleus sampling** | 核采样 | 保留累计概率达到阈值p的最小候选集合再采样 | [9][25] |
| **Greedy decoding** | 贪婪解码 | 每一步都选概率最高的token（HF中 `do_sample=False` 对应不采样的路径） | [9] |
| **Beam search** | 束搜索 | 维护多个候选序列并扩展比较的搜索式解码（HF中 `num_beams` 等参数） | [9] |
| **use_cache / KV cache** | 键值缓存 | 复用过去注意力的K/V来加速逐token解码 | [9] |
| **max_completion_tokens** | 最大生成token数上限 | OpenAI API参数，包含"可见输出token + reasoning tokens" | [3] |
| **stop_strings** | 停止串 | 一旦生成到指定字符串就终止 | [9] |
| **logprobs** | 输出token对数概率 | OpenAI API可选择返回输出token的log probabilities | [3] |
| **logit_bias** | logit偏置 | 通过对特定token的logits加偏置来提高/降低其出现概率 | [3] |
| **Streaming** | 流式输出 | 边生成边返回结果；Structured Outputs也讨论了与streaming结合的解析 | [4] |

---

## 7. 工具调用、结构化输出与Agent系统（Tools, Structured Outputs, Agents）

| 术语 | 中文 | 定义 | 来源 |
|------|------|------|------|
| **Tool** | 工具 | 模型可调用的外部能力（函数/检索/计算等），在API中以"tools"等机制接入 | [3][4] |
| **Function calling** | 函数调用 | 让模型输出可解析的函数名与参数，以便调用外部函数/工具（OpenAI文档将其与Structured Outputs关联） | [4][3] |
| **tool_choice** | 工具选择控制 | OpenAI 文档中提到取代旧 `function_call` 的控制方式（强制/自动/不调用等语义） | [3] |
| **Parallel tool calls** | 并行工具调用 | OpenAI API参数 `parallel_tool_calls` 用于启用并行函数调用 | [3] |
| **Structured Outputs** | 结构化输出 | 保证模型输出严格符合给定JSON Schema（比"只保证是JSON"的JSON mode更强） | [4] |
| **JSON Schema** | JSON模式约束 | Structured Outputs 的约束语言基础 | [4] |
| **JSON mode** | JSON模式 | 保证输出是合法JSON，但不保证满足特定schema | [4] |
| **Agent** | 智能体 | 让模型不仅"回答"，还会"计划-调用工具-观察结果-再行动"的系统形态；ReAct是典型范式 | [17] |
| **MAS** | Multi-Agent System | 多智能体系统；多个代理通过分工/对话协作完成任务 | [26] ⚠️ |
| **MCP** | Model Context Protocol | 连接AI应用与外部数据源/工具/工作流的开源标准（"像AI应用的USB-C接口"） | [22] |
| **MCP Client / Server** | 客户端/服务器 | MCP架构里，client连接server，server暴露数据源/工具 | [22] |
| **Skills** | 技能 | 常指"可复用的工具组合/子流程封装" | ⚠️ |

---

## 8. 安全、评测与可靠性（Safety, Eval, Reliability）

| 术语 | 中文 | 定义 | 来源 |
|------|------|------|------|
| **Prompt Injection** | 提示注入 | 当用户输入以非预期方式改变模型行为/输出，可能导致泄露信息、越权调用工具等 | [23] |
| **Direct vs Indirect Prompt Injection** | 直接/间接注入 | OWASP区分：直接来自用户输入；间接来自外部内容（网页/文件）被模型读取后触发 | [23] |
| **Jailbreaking** | 越狱 | OWASP指出它是 prompt injection 的一种，使模型无视安全协议 | [23] |
| **Evals / Benchmarking** | 评测/基准测试 | OpenAI建议构建 evals 来监控提示随迭代或模型升级的行为变化 | [1] |
| **Explicit refusals** | 可检测拒答 | Structured Outputs 的一个收益是"安全拒答可被程序化检测" | [4] |
| **Safety identifier** | 安全标识 | OpenAI API参数，用于帮助检测潜在违规用户（建议哈希化避免发送识别信息） | [3] |

---

## 待验证术语（Evidence Insufficient）

> ⚠️ 标记的术语在本次检索中未能获取到权威一句话定义，建议通过以下路径验证：

| 术语 | 建议验证路径 |
|------|-------------|
| Pretraining | 查 OpenAI/Anthropic/HuggingFace 官方 Glossary |
| SFT | 查 InstructGPT 论文或 HuggingFace alignment handbook |
| Reward model | 查 RLHF 经典论文（InstructGPT）摘要或引言 |
| Hallucination | 查 ACM/IEEE 顶会教程或机构白皮书 |
| Chunking | 查 LangChain/LlamaIndex 官方文档 |
| MAS | 查 AutoGen/CrewAI 等多智能体框架论文 |
| Skills | 术语在不同框架含义不一，需具体上下文 |

---

## 参考文献

| # | 来源 | 说明 | 链接 |
|---|------|------|------|
| [1] | OpenAI Platform | Prompt engineering 定义、指令权威层级、建议做eval、model snapshot | [Prompt engineering](https://platform.openai.com/docs/guides/prompt-engineering) |
| [2] | OpenAI Platform | 推理模型提示方式、developer message说明 | [Reasoning best practices](https://platform.openai.com/docs/guides/reasoning-best-practices) |
| [3] | OpenAI API Reference | max_completion_tokens、logit_bias/logprobs、tool参数等 | [Chat Completions](https://platform.openai.com/docs/api-reference/chat) |
| [4] | OpenAI Platform | Structured Outputs定义、JSON Schema/JSON mode | [Structured model outputs](https://platform.openai.com/docs/guides/structured-outputs) |
| [5] | OpenAI Platform | embedding用法、余弦相似度检索示例 | [Vector embeddings](https://platform.openai.com/docs/guides/embeddings) |
| [6] | OpenAI Help Center | token定义 | [What are tokens](https://help.openai.com/en/articles/4936856-what-are-tokens-and-how-to-count-them) |
| [7] | GitHub | tiktoken与BPE tokenizer说明 | [openai/tiktoken](https://github.com/openai/tiktoken) |
| [8] | ACL Anthology | 子词单元/BPE思想的学术来源（2016） | [Subword Units PDF](https://aclanthology.org/P16-1162.pdf) |
| [9] | Hugging Face | temperature/top_k/top_p/use_cache/beam等参数定义 | [Text Generation](https://hugging-face.cn/docs/transformers/main_classes/text_generation) |
| [10] | NeurIPS 2017 | Transformer/Attention术语来源 | [Attention Is All You Need](https://proceedings.neurips.cc/paper_files/paper/2017/file/3f5ee243547dee91fbd053c1c4a845aa-Paper.pdf) |
| [11] | NeurIPS 2020 | few-shot / in-context learning来源 | [GPT-3 Paper](https://proceedings.neurips.cc/paper/2020/file/1457c0d6bfcb4967418bfb8ac142f64a-Paper.pdf) |
| [12] | arXiv 2020 | scaling laws来源 | [Scaling Laws](https://arxiv.org/abs/2001.08361) |
| [13] | arXiv 2022 | CoT定义来源 | [Chain-of-Thought](https://arxiv.org/abs/2201.11903) |
| [14] | arXiv 2022 | Self-Consistency来源 | [Self-Consistency](https://arxiv.org/abs/2203.11171) |
| [15] | arXiv 2023 | ToT来源 | [Tree of Thoughts](https://arxiv.org/abs/2305.10601) |
| [16] | arXiv 2020 | RAG来源 | [RAG Paper](https://arxiv.org/abs/2005.11401) |
| [17] | arXiv 2022 | ReAct/Agent推理+行动范式 | [ReAct](https://arxiv.org/abs/2210.03629) |
| [18] | arXiv 2022 | RLHF/用人类反馈微调路径 | [InstructGPT](https://arxiv.org/abs/2203.02155) |
| [19] | arXiv 2023 | DPO定义来源 | [DPO](https://arxiv.org/abs/2305.18290) |
| [20] | arXiv 2021 | LoRA定义来源 | [LoRA](https://arxiv.org/abs/2106.09685) |
| [21] | arXiv 2023 | 反思式代理来源 | [Reflexion](https://arxiv.org/abs/2303.11366) |
| [22] | MCP官网 | MCP定义、client/server概念 | [Model Context Protocol](https://modelcontextprotocol.io/introduction) |
| [23] | OWASP | prompt injection、direct/indirect、jailbreak关系 | [LLM01 Prompt Injection](https://genai.owasp.org/llmrisk/llm01-prompt-injection/) |
| [24] | Microsoft Learn | system message定义与用途（2025-12-06更新） | [System message design](https://learn.microsoft.com/en-us/azure/ai-foundry/openai/concepts/advanced-prompt-engineering) |
| [25] | arXiv 2019 | top-p/nucleus sampling学术来源 | [Neural Text Degeneration](https://arxiv.org/abs/1904.09751) |
| [26] | AutoGen等 | MAS验证线索（证据待补充） | - |

# 8 Different Specialized AI Models

> **Note:** These labels describe different AI architectures or model
> concepts. Some terms, such as **LCM** and **LAM**, are used
> differently across projects and research communities. The diagrams
> below follow the structure shown in the reference image.

------------------------------------------------------------------------

## 1. LLM --- Large Language Model

### Description

A **Large Language Model (LLM)** processes text by converting input into
tokens, generating embeddings, passing them through transformer layers,
and producing an output.

LLMs are primarily designed for understanding and generating language.

### Typical use cases

-   Chatbots and conversational assistants
-   Code generation and code review
-   Text generation and rewriting
-   Summarization
-   Translation
-   Question answering
-   Information extraction
-   Reasoning over text

``` mermaid
flowchart TB
    A["Input"] --> B["Tokenization"]
    B --> C["Embedding"]
    C --> D["Transformer"]
    D --> E["Output"]
```

------------------------------------------------------------------------

## 2. LCM --- Large Concept Model

### Description

A **Large Concept Model (LCM)** is presented as operating at a higher
semantic level than traditional token-by-token language modeling.
Instead of directly operating only on individual tokens, it can process
larger semantic units or concepts.

The reference architecture shows sentence segmentation, SONAR
embeddings, diffusion, advanced patterning, hidden processing, and
quantization.

### Typical use cases

-   Long-form semantic processing
-   High-level document understanding
-   Concept-level generation
-   Multilingual semantic representation
-   Large-context reasoning
-   Research into alternatives to token-level generation

``` mermaid
flowchart TB
    A["Input"] --> B["Sentence Segmentation"]
    B --> C["SONAR Embedding"]
    C --> D["Diffusion"]

    D --> E["Advanced Patterning"]
    D --> F["Hidden Process"]

    E --> G["Quantization"]
    F --> G

    G --> H["Output"]
```

------------------------------------------------------------------------

## 3. LAM --- Large Action Model

### Description

A **Large Action Model (LAM)** focuses on transforming user intent into
actions. Instead of only generating text, it can interpret an objective,
break it into tasks, plan actions, interact with tools or systems, use
memory, and incorporate feedback.

This makes the concept particularly relevant to AI agents.

### Typical use cases

-   Computer-use agents
-   RPA automation
-   Browser automation
-   Multi-step business workflows
-   API orchestration
-   Autonomous task execution
-   Tool-using AI agents
-   Planning and decision workflows

``` mermaid
flowchart TB
    A["Input Processing"] --> B["Perception System"]
    B --> C["Intent Recognition"]
    C --> D["Task Breakdown"]

    D --> E["Action Planning"]
    D --> F["Memory Integration"]
    C --> G["Neuro-Symbolic Integration"]

    G --> F
    E --> H["Quantization"]
    F --> H
    D --> H

    H --> I["Feedback Integration"]
```

------------------------------------------------------------------------

## 4. MoE --- Mixture of Experts

### Description

A **Mixture of Experts (MoE)** model contains multiple specialized
expert networks. A router determines which experts should process a
given input, typically activating only a subset of them.

This allows a model to have a very large total parameter count while
using only part of the model for each inference step.

### Typical use cases

-   Large-scale LLMs
-   Efficient inference at high parameter counts
-   Specialized reasoning paths
-   Multilingual models
-   Code and general-language specialization
-   Large production AI systems

``` mermaid
flowchart TB
    A["Input"] --> B["Router Mechanism"]

    B --> C["Expert 1"]
    B --> D["Expert 2"]
    B --> E["Expert 3"]
    B --> F["Expert 4"]

    C --> G["Top-K Selection"]
    D --> G
    E --> G
    F --> G

    G --> H["Weighted Combination"]
    H --> I["Output"]
```

------------------------------------------------------------------------

## 5. VLM --- Vision-Language Model

### Description

A **Vision-Language Model (VLM)** combines visual and textual inputs. An
image can be processed by a vision encoder while text is processed by a
text encoder. Their representations are combined before being passed to
a language model.

VLMs allow an AI system to reason about images using natural language.

### Typical use cases

-   Image understanding
-   Visual question answering
-   OCR and document understanding
-   Image captioning
-   Screenshot analysis
-   UI understanding
-   Product recognition
-   Visual inspection
-   Multimodal assistants

``` mermaid
flowchart TB
    A["Image Input"] --> B["Vision Encoder"]
    C["Text Input"] --> D["Text Encoder"]

    B --> E["Projection Interface"]
    D --> E

    E --> F["Multimodal Processor"]
    F --> G["Language Model"]
    G --> H["Output Generation"]
```

------------------------------------------------------------------------

## 6. SLM --- Small Language Model

### Description

A **Small Language Model (SLM)** is a smaller, more resource-efficient
language model designed to provide useful language capabilities with
substantially lower compute and memory requirements than very large
models.

The reference architecture emphasizes compact tokenization, an efficient
transformer, quantization, memory optimization, and edge deployment.

### Typical use cases

-   Local AI assistants
-   On-device AI
-   Edge computing
-   Mobile applications
-   Embedded systems
-   Private/offline inference
-   Low-latency applications
-   Systems with limited GPU/CPU resources

``` mermaid
flowchart TB
    A["Input Processing"] --> B["Compact Tokenization"]
    B --> C["Efficient Transformer"]

    C --> D["Model Quantization"]
    C --> E["Memory Optimization"]

    D --> F["Edge Deployment"]
    E --> F

    F --> G["Output Generation"]
```

------------------------------------------------------------------------

## 7. MLM --- Masked Language Model

### Description

A **Masked Language Model (MLM)** learns language by hiding or masking
tokens and training the model to predict the missing information using
context from both sides.

This approach is strongly associated with bidirectional language
understanding rather than purely left-to-right generation.

### Typical use cases

-   Text classification
-   Semantic embeddings
-   Named entity recognition
-   Search and information retrieval
-   Document understanding
-   Sentence similarity
-   Language representation learning
-   Pretraining encoder-based models

``` mermaid
flowchart TB
    A["Text Input"] --> B["Token Masking"]
    B --> C["Embedding Layer"]

    C --> D["Left Context"]
    C --> E["Right Context"]

    D --> F["Bidirectional Attention"]
    E --> F

    F --> G["Masked Token Prediction"]
    G --> H["Feature Representation"]
```

------------------------------------------------------------------------

## 8. SAM --- Segment Anything Model

### Description

**SAM (Segment Anything Model)** is a computer-vision model designed for
image segmentation. It combines an image encoder with a prompt encoder,
allowing a user or another system to provide prompts that guide which
regions of an image should be segmented.

Prompts can represent points, boxes, masks, or other supported guidance
mechanisms depending on the SAM implementation.

### Typical use cases

-   Image segmentation
-   Object isolation
-   Background removal
-   Image editing
-   Dataset annotation
-   Computer vision pipelines
-   Object measurement
-   Visual inspection
-   Region extraction
-   Robotics and perception

``` mermaid
flowchart TB
    A["Prompt Input"] --> B["Prompt Encoder"]
    C["Image Input"] --> D["Image Encoder"]

    B --> E["Image Embedding"]
    D --> E

    E --> F["Mask Decoder"]
    E --> G["Feature Correlation"]

    F --> H["Segmentation Output"]
    G --> H
```

------------------------------------------------------------------------

# Quick Comparison

  --------------------------------------------------------------------------------------------------
  Model          Main Focus           Typical Input         Main Output            Common
                                                                                   Applications
  -------------- -------------------- --------------------- ---------------------- -----------------
  **LLM**        Language             Text                  Text/tokens            Chat, coding,
                                                                                   reasoning

  **LCM**        Concepts/semantics   Text/concepts         Concepts or generated  Semantic
                                                            content                generation,
                                                                                   long-context
                                                                                   processing

  **LAM**        Actions              Intent/instructions   Actions/workflows      Agents, RPA,
                                                                                   automation

  **MoE**        Specialized experts  Text or multimodal    Model output           Large efficient
                                      data                                         models

  **VLM**        Vision + language    Images + text         Text/analysis          Image
                                                                                   understanding,
                                                                                   visual agents

  **SLM**        Efficient language   Text                  Text                   Edge/local AI

  **MLM**        Bidirectional        Masked text           Predictions/features   Embeddings,
                 language                                                          classification,
                 representation                                                    NER

  **SAM**        Image segmentation   Image + prompt        Segmentation mask      Computer vision,
                                                                                   annotation
  --------------------------------------------------------------------------------------------------

------------------------------------------------------------------------

# How They Relate

These concepts are not necessarily mutually exclusive. A modern AI
system can combine several of them.

For example:

``` mermaid
flowchart LR
    A["Image"] --> B["VLM"]
    B --> C["LLM"]
    C --> D["LAM"]
    D --> E["Tools / APIs / Browser"]

    A --> F["SAM"]
    F --> G["Segmented Regions"]
    G --> B

    C --> H["MoE"]
    H --> C
```

A practical AI agent could therefore use:

-   **VLM** to understand what is visible.
-   **SAM** to isolate specific objects or regions.
-   **LLM** to reason about language and context.
-   **MoE** to route computation through specialized experts.
-   **LAM** to turn the reasoning into actions.
-   **SLM** when the task needs to run locally or on constrained
    hardware.
-   **MLM** for embeddings, classification, retrieval, or other
    language-understanding components.
-   **LCM** for concept-level or semantic processing where that
    architecture is appropriate.

# How Attention Works

> **Attention finds relevant words using similarity.**
>
> The core idea of self-attention is that each token can look at all
> other tokens and assign more weight to the most relevant ones.

------------------------------------------------------------------------

## 1. Input Tokens

A sentence is first split into individual tokens.

Example:

``` text
The | cat | sat | on | the | mat
```

Each token is converted into a numerical **embedding vector** of
dimension `d`.

``` mermaid
flowchart LR
    A["The"] --> A1["Embedding<br/>(d)"]
    B["cat"] --> B1["Embedding<br/>(d)"]
    C["sat"] --> C1["Embedding<br/>(d)"]
    D["on"] --> D1["Embedding<br/>(d)"]
    E["the"] --> E1["Embedding<br/>(d)"]
    F["mat"] --> F1["Embedding<br/>(d)"]
```

### Key idea

The model does not operate directly on words. It operates on numerical
representations of the tokens.

------------------------------------------------------------------------

## 2. Create Q, K, V

The input matrix `X` contains the token embeddings.

Three learned linear transformations convert `X` into:

-   **Q --- Query:** What is this token looking for?
-   **K --- Key:** What information does this token contain that others
    might be looking for?
-   **V --- Value:** What information should actually be passed forward?

Mathematically:

``` text
Q = XW_Q
K = XW_K
V = XW_V
```

Where:

``` text
X   = n × d
Q   = n × d_k
K   = n × d_k
V   = n × d_v
```

``` mermaid
flowchart LR
    X["Input X<br/>(n × d)"]

    X --> WQ["W_Q"]
    X --> WK["W_K"]
    X --> WV["W_V"]

    WQ --> Q["Queries Q<br/>(n × d_k)"]
    WK --> K["Keys K<br/>(n × d_k)"]
    WV --> V["Values V<br/>(n × d_v)"]
```

------------------------------------------------------------------------

## 3. Compute Similarity

The model compares every query against every key.

This is done using a matrix multiplication:

``` text
Scores = QKᵀ
```

Dimensions:

``` text
Q   = n × d_k
Kᵀ  = d_k × n

QKᵀ = n × n
```

The resulting `n × n` matrix contains a similarity score for every pair
of tokens.

``` mermaid
flowchart LR
    Q["Q<br/>(n × d_k)"]
    KT["Kᵀ<br/>(d_k × n)"]

    Q --> M["Matrix Multiplication<br/>QKᵀ"]
    KT --> M

    M --> S["Scores<br/>(n × n)"]
```

### Key idea

For every token, the model asks:

> "How relevant is every other token to me?"

------------------------------------------------------------------------

## 4. Scale & Softmax

The raw attention scores are scaled by the square root of the key
dimension:

``` text
Scaled Scores = QKᵀ / √d_k
```

Then **softmax** is applied row-wise.

``` text
Attention Weights = softmax(QKᵀ / √d_k)
```

This converts the scores into normalized weights.

``` mermaid
flowchart LR
    A["Scores<br/>(n × n)"]
    B["÷ √d_k"]
    C["Softmax<br/>(row-wise)"]
    D["Attention Weights<br/>(n × n)"]

    A --> B --> C --> D
```

### Interpretation

A higher attention weight means:

> "Pay more attention to this token."

A lower weight means:

> "This token is less relevant for the current token."

------------------------------------------------------------------------

## 5. Weighted Sum of Values

The attention weights are multiplied by the value matrix:

``` text
Context = Attention Weights × V
```

Dimensions:

``` text
Attention Weights = n × n
V                 = n × d_v

Context           = n × d_v
```

``` mermaid
flowchart LR
    A["Attention Weights<br/>(n × n)"]
    B["V<br/>(n × d_v)"]

    A --> M["Matrix Multiplication"]
    B --> M

    M --> C["Context Vectors<br/>(n × d_v)"]
```

### Key idea

The attention weights determine how much information from each token
should contribute to the new context representation.

------------------------------------------------------------------------

## 6. What It Learns --- Example

Consider:

``` text
The cat sat on the mat
```

When processing **"cat"**, the attention mechanism can assign different
weights to the other words.

For example, "cat" may pay significant attention to words such as:

``` text
The
cat
sat
```

while assigning lower weights to less relevant tokens.

``` mermaid
flowchart TB
    A["The cat sat on the mat"]

    A --> B["Attention for 'cat'"]

    B --> C["The"]
    B --> D["cat"]
    B --> E["sat"]
    B --> F["on"]
    B --> G["the"]
    B --> H["mat"]

    C --> I["Attention weight"]
    D --> J["Attention weight"]
    E --> K["Attention weight"]
    F --> L["Attention weight"]
    G --> M["Attention weight"]
    H --> N["Attention weight"]
```

The exact attention pattern is learned by the model and depends on the
context, layer, head, and learned parameters.

### Important point

Attention is not simply a fixed dictionary of word relationships.

The relationships are computed dynamically from the representations of
the tokens.

------------------------------------------------------------------------

## 7. Final Output

The context vectors can be passed through a final linear projection.

``` text
Output = Context × W_O
```

The resulting representation has the model's desired hidden dimension:

``` text
Context = n × d_v
Output  = n × d_model
```

``` mermaid
flowchart LR
    A["Context Vectors<br/>(n × d_v)"]
    B["Linear Layer<br/>W_O"]
    C["Output<br/>(n × d_model)"]

    A --> B --> C
```

This output can then be passed to subsequent components of the
Transformer architecture.

------------------------------------------------------------------------

## 8. Big Picture

The complete simplified self-attention process is:

``` mermaid
flowchart TB
    A["Input Tokens<br/>X"] --> B["Create Q, K, V"]

    B --> C["Q"]
    B --> D["K"]
    B --> E["V"]

    C --> F["QKᵀ"]
    D --> F

    F --> G["Scale<br/>÷ √d_k"]
    G --> H["Softmax"]
    H --> I["Attention Weights"]

    I --> J["Weighted Sum"]
    E --> J

    J --> K["Context-Aware Representations"]
    K --> L["Linear Projection"]
    L --> M["Output"]
```

### The core formula

The complete scaled dot-product attention operation is:

``` text
Attention(Q, K, V)
    = softmax(QKᵀ / √d_k)V
```

------------------------------------------------------------------------

# Intuition

You can think of self-attention as every word asking:

``` text
"What other words should I pay attention to
in order to understand my meaning in this sentence?"
```

For each token:

1.  **Query** asks what information it needs.
2.  **Key** describes what each token can provide.
3.  **Similarity** determines how relevant each key is to the query.
4.  **Softmax** converts relevance scores into attention weights.
5.  **Values** provide the actual information.
6.  **Weighted sum** creates a context-aware representation.

``` mermaid
flowchart LR
    A["Token"] --> B["Query"]
    B --> C["Compare with Keys"]
    C --> D["Similarity Scores"]
    D --> E["Softmax"]
    E --> F["Attention Weights"]
    F --> G["Weighted Values"]
    G --> H["Context-Aware Token"]
```

------------------------------------------------------------------------

# One-Line Summary

> **Self-attention lets every token dynamically decide which other
> tokens are relevant, then combines their information into a
> context-aware representation.**

------------------------------------------------------------------------

# Full Mathematical Summary

``` text
Input:
    X

Create projections:
    Q = XW_Q
    K = XW_K
    V = XW_V

Calculate similarity:
    Scores = QKᵀ

Scale:
    Scaled Scores = QKᵀ / √d_k

Normalize:
    Weights = softmax(QKᵀ / √d_k)

Aggregate information:
    Context = Weights V

Project:
    Output = Context W_O
```

Or, compactly:

``` text
Attention(Q, K, V)
    = softmax(QKᵀ / √d_k)V
```

# RAG Project Structure

Example project structure for a Retrieval-Augmented Generation (RAG) application.

## Project Structure

```text
rag-project/
│
├── README.md
├── requirements.txt
├── .env
├── .gitignore
├── config.yaml
│
├── src/
│   ├── ingestion/
│   │   ├── __init__.py
│   │   └── loader.py
│   │
│   ├── chunking/
│   │   ├── __init__.py
│   │   └── chunker.py
│   │
│   ├── embeddings/
│   │   ├── __init__.py
│   │   └── embedder.py
│   │
│   ├── vectordb/
│   │   ├── __init__.py
│   │   └── vector_store.py
│   │
│   ├── retrieval/
│   │   ├── __init__.py
│   │   └── retriever.py
│   │
│   ├── prompts/
│   │   ├── __init__.py
│   │   └── prompt_templates.py
│   │
│   ├── llm/
│   │   ├── __init__.py
│   │   └── llm_client.py
│   │
│   ├── api/
│   │   ├── __init__.py
│   │   └── routes.py
│   │
│   └── utils/
│       ├── __init__.py
│       └── helpers.py
│
├── tests/
│   └── test_app.py
│
├── logs/
│   └── app.log
│
└── main.py
```

## Project Components

| Path | Purpose |
|---|---|
| `README.md` | Project description, setup instructions, how to run the application, and architecture documentation. |
| `requirements.txt` | List of Python dependencies required by the project. |
| `.env` | Stores API keys and other sensitive configuration. Should not be committed to Git. |
| `.gitignore` | Specifies files and directories that Git should ignore. |
| `config.yaml` | Application configuration such as models, chunk size, database settings, etc. |
| `src/ingestion/` | Loads source data from PDFs, CSVs, websites, and other sources. |
| `src/chunking/` | Splits loaded text into smaller chunks suitable for embedding and retrieval. |
| `src/embeddings/` | Converts text chunks into vector embeddings. |
| `src/vectordb/` | Handles vector database operations such as ChromaDB, Pinecone, or FAISS. |
| `src/retrieval/` | Retrieves relevant chunks using similarity search. |
| `src/prompts/` | Stores prompt templates used by the application. |
| `src/llm/` | Handles LLM interactions with providers such as OpenAI, Claude, Gemini, etc. |
| `src/api/` | Defines API endpoints using frameworks such as FastAPI or Flask. |
| `src/utils/` | Contains helper functions and common utilities. |
| `tests/` | Unit tests and integration tests. |
| `logs/` | Application logs used for debugging and monitoring. |
| `main.py` | Main entry point for the application. |

## RAG Flow

The structure maps naturally to the typical RAG pipeline:

```text
                    ┌──────────────┐
                    │ Source Data  │
                    │ PDF / CSV /  │
                    │ Website /... │
                    └──────┬───────┘
                           │
                           ▼
                    ┌──────────────┐
                    │  Ingestion   │
                    │   loader.py  │
                    └──────┬───────┘
                           │
                           ▼
                    ┌──────────────┐
                    │   Chunking   │
                    │  chunker.py  │
                    └──────┬───────┘
                           │
                           ▼
                    ┌──────────────┐
                    │  Embeddings  │
                    │  embedder.py │
                    └──────┬───────┘
                           │
                           ▼
                    ┌──────────────┐
                    │   Vector DB  │
                    │vector_store.py│
                    └──────┬───────┘
                           │
                     Similarity Search
                           │
                           ▼
                    ┌──────────────┐
                    │  Retrieval   │
                    │ retriever.py │
                    └──────┬───────┘
                           │
                    Relevant Context
                           │
                           ▼
                    ┌──────────────┐
                    │    Prompt    │
                    │   Templates  │
                    └──────┬───────┘
                           │
                           ▼
                    ┌──────────────┐
                    │     LLM      │
                    │ llm_client.py│
                    └──────┬───────┘
                           │
                           ▼
                    ┌──────────────┐
                    │    Answer    │
                    └──────────────┘
```

## Suggested Responsibility Boundaries

| Component | Responsibility |
|---|---|
| `ingestion` | Get documents into the system |
| `chunking` | Split documents |
| `embeddings` | Convert chunks to vectors |
| `vectordb` | Store and query vectors |
| `retrieval` | Select relevant context |
| `prompts` | Construct prompts |
| `llm` | Generate responses |
| `api` | Expose functionality externally |
| `utils` | Shared helpers |
| `tests` | Verify behavior |

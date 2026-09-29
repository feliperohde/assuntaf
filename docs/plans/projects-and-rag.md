# Plano: Projetos + Memória RAG sobre reuniões (Meetily / assunta)

## Contexto

Hoje o app grava, transcreve e resume reuniões isoladas. Não há forma de **perguntar** sobre o histórico
("o que foi falado na reunião X?", "em que dia Fulano falou Y?", "por que o ticket ABC-123 está bloqueado?")
nem de **organizar** reuniões por projeto. A busca atual (`TranscriptsRepository::search_transcripts`,
`src-tauri/src/database/repositories/transcript.rs:87`) é um `LIKE %q%` sem ranking, sem semântica e sem escopo.

Objetivo: (1) introduzir **Projetos** como contêiner de tudo (reuniões, transcrições, resumos, índice RAG,
entidades); (2) indexar cada reunião com embeddings locais (Ollama) + índice lexical; (3) identificar
**quem falou** via diarização automática; (4) oferecer um "Pergunte ao projeto" com respostas citando
reunião, data, falante e minuto do áudio.

Decisões já tomadas com o usuário: embeddings via **Ollama local**; **diarização automática**; tickets
extraídos **só das reuniões** agora, com integrações (Jira/Linear) no futuro.

---

## Arquitetura RAG recomendada

**Hybrid RAG + camada de entidades/fatos ("Graph-lite") + roteador agêntico leve (planner com ferramentas),
com checagem de evidência estilo CRAG.** Não recomendo Multi-Agent RAG — é custo e latência sem ganho para
um corpus pessoal/local.

Por que cada peça (mapeado às notas em `docs/notes/`):

| Pergunta típica | O que resolve | Técnica |
|---|---|---|
| "o que foi falado na reunião de terça sobre deploy?" | filtro de data/reunião + semântica | **Vector + filtros de metadados** |
| "ABC-123", nomes próprios, siglas | match exato que embedding erra | **BM25 (SQLite FTS5)** → fusão RRF (*Hybrid RAG*) |
| "em que dia Fulano falou X?" | falante + data como metadado do chunk | **Diarização** + filtro `speaker_id` |
| "por que o ticket X está bloqueado?" | estado mais recente + histórico | **Tabela de entidades/fatos** extraídos (*Graph RAG* simplificado, relacional) |
| perguntas compostas / temporais | decidir qual ferramenta usar | **Planner agêntico** (1 chamada LLM → JSON de plano; *Agentic RAG*) |
| evitar alucinação | "não encontrei" em vez de inventar | **Grade de evidência** antes de responder (*CRAG*, sem web fallback) |

Fluxo de consulta:

```
Pergunta ──► Planner (LLM, JSON): {intent, filtros{project, date_range, meeting, speaker}, tools[]}
               │
     ┌─────────┼──────────────┬───────────────────┐
     ▼         ▼              ▼                   ▼
 vector_search  bm25_search   entity_lookup     meeting_summary
 (cosine,       (FTS5)        (tickets/pessoas/  (resumos prontos)
  escopo projeto)              decisões + fatos)
     └────► RRF fusion ◄──────┘
              ▼
        Grade evidência (score mínimo / LLM leve) ── insuficiente ──► "não encontrado" (ou 1 retry com query reescrita)
              ▼
        LLM responde com citações [reunião · data · falante · mm:ss] → clicável → abre reunião no timestamp
```

Armazenamento: **tudo no SQLite existente** (mesmo `DatabaseManager`, mesmas migrations sqlx).
- Vetores como `BLOB` (f32 little-endian) + busca por **cosine brute-force em Rust**, sempre filtrada por
  `project_id`. Escala pessoal (≈ 100 reuniões × 150 chunks × 1024 dims) roda em poucos ms; evita extensão
  nativa. Trait `VectorStore` permite trocar por `sqlite-vec` depois se crescer.
- Lexical via **FTS5** (já incluso no SQLite bundled do `libsqlite3-sys`) — verificar na Fase 2.

Modelo de embedding padrão: **`bge-m3`** no Ollama (multilíngue, ótimo em PT-BR, 1024 dims, 8k contexto);
alternativa leve `nomic-embed-text`. Guardar `embedding_model` + `dims` por chunk; trocar de modelo = reindexar.

---

## Fases de implementação

### Fase 1 — Projetos (fundação)

Migration `migrations/2026xxxx_add_projects.sql`:
- `projects(id, name, description, context_md, glossary, ticket_patterns, color, created_at, updated_at, archived)`
  - `context_md`: características do projeto (objetivo, stack, time) — injetado em prompts de resumo e RAG.
  - `glossary`: termos/nomes — também usado como *initial prompt* do Whisper para melhorar transcrição.
  - `ticket_patterns`: regex dos IDs (ex.: `ABC-\d+`) usados na extração de entidades.
- `project_members(id, project_id, name, role, email, voiceprint BLOB NULL)`.
- `ALTER TABLE meetings ADD COLUMN project_id TEXT REFERENCES projects(id)`; criar projeto "Geral" e
  associar reuniões existentes a ele (backfill na própria migration).

Rust:
- `database/repositories/project.rs` (CRUD, seguindo o padrão de `meeting.rs`), modelos em `database/models.rs`.
- Comandos Tauri novos em `src/projects/commands.rs`, registrados em `lib.rs` (`generate_handler!`, ~l.612).
- `api_save_transcript` (`src/api/api.rs` ~l.976) e `TranscriptsRepository::save_transcript` recebem `project_id`.
- `MeetingsRepository::get_meetings` aceita filtro por projeto; `search_transcripts` idem.

Frontend:
- `ProjectContext` (novo, em `src/contexts/`) com projeto ativo persistido via `tauri-plugin-store`.
- Seletor de projeto na Sidebar (`components/Sidebar/SidebarProvider.tsx` filtra lista de reuniões).
- Página `src/app/projects/` (criar/editar projeto, membros, glossário, padrões de ticket).
- Início de gravação e `ImportAudio` usam o projeto ativo; mover reunião entre projetos (reindexa).
- Resumo (`src/summary/service.rs`) inclui `context_md` do projeto no prompt.

### Fase 2 — Pipeline de indexação (ingestion → chunking → embeddings)

Novo módulo `src-tauri/src/rag/`, espelhando `docs/notes/rag-project-structure.md`:
```
rag/
├── mod.rs, commands.rs        # comandos Tauri: reindex_meeting, reindex_project, rag_status, ask_project
├── ingestion.rs               # carrega segmentos + resumo + notas de uma reunião
├── chunker.rs                 # chunking por janela de tempo
├── embeddings/{mod.rs, ollama.rs}   # trait EmbeddingProvider; impl Ollama (/api/embed, batch)
├── store.rs                   # VectorStore (BLOB + cosine) e FTS5
├── retriever.rs               # vector + bm25 + RRF + filtros
├── entities.rs                # extração de tickets/decisões/ações (Fase 4)
├── planner.rs, prompts.rs     # planner agêntico e templates
└── indexer.rs                 # fila de jobs em background + eventos de progresso
```

Migration `add_rag_index.sql`:
- `rag_chunks(id, project_id, meeting_id, kind['transcript'|'summary'|'notes'|'action_item'], text,
  speaker_ids JSON, start_time, end_time, meeting_date, embedding BLOB, embedding_model, dims, created_at)`
  + índices `(project_id)`, `(meeting_id)`.
- `rag_chunks_fts` (FTS5, `content='rag_chunks'`, tokenizer `unicode61 remove_diacritics 2` para PT-BR) + triggers.
- `rag_index_jobs(meeting_id, status, error, updated_at)` para retomada/reprocessamento.

Chunking (transcrição é conversa, não documento):
- Agrupar segmentos consecutivos em janelas de **~60–90 s / ~300–500 tokens**, overlap de 1–2 segmentos,
  **quebrando em troca longa de falante**. Texto do chunk prefixado com cabeçalho contextual
  (`[Projeto X · Reunião "Daily" · 2026-09-22 · Ana, Bruno]`) — melhora muito o recall de perguntas temporais.
- Chunks adicionais de nível alto: resumo da reunião, action items e notas (`meeting_notes`) → recuperação
  hierárquica ("o que foi falado na reunião tal" usa o resumo primeiro).

Gatilhos: ao fim do resumo/salvamento da reunião (hook em `api_save_transcript` e no término do summary),
edição de transcrição/falante, mudança de projeto. Indexação roda em `tokio::spawn` com fila, emite eventos
`rag-index-progress`; falha do Ollama → job fica `pending` e é reprocessado. Botão "Reindexar projeto".

Settings: provider/modelo de embedding na tabela `settings` (nova migration), reutilizando `ollama_endpoint`
e helpers de `src/ollama/ollama.rs` (listar/baixar modelo — `pull_ollama_model` já existe) na UI de settings.

### Fase 3 — Diarização automática (quem falou)

Observação: `src/audio/stt.rs` com pyannote é código legado **não compilado** (não está em `audio/mod.rs`,
depende de crates do screenpipe) — serve só de referência. Implementação nova, **offline, pós-gravação**:

- Novo módulo `src-tauri/src/diarization/` usando **`ort`** (já dependência): modelo de segmentação
  `pyannote segmentation-3.0` (ONNX) + embedding de voz (`wespeaker`/3D-Speaker ONNX) + clustering
  aglomerativo por cosine. Avaliar crate `pyannote-rs` (usa `ort`) antes de escrever do zero — checar
  compatibilidade com `ort = 2.0.0-rc.10` e o `load-dynamic` do Windows.
- Download dos modelos no mesmo padrão dos gerenciadores existentes (`WhisperModelManager`/`ParakeetModelManager`).
- Roda sobre o áudio salvo em `meetings.folder_path` após a gravação; mapeia clusters aos segmentos por
  sobreposição de tempo (`audio_start_time`/`audio_end_time`). Segmentos `speaker='mic'` → dono do app.
- **Identificação**: centroides comparados às `project_members.voiceprint`; acima do limiar → nome do membro,
  senão "Falante 1/2…". UI na tela da reunião para renomear/atribuir falante a um membro — isso grava/atualiza
  o voiceprint (aprendizado contínuo) e dispara reindexação da reunião.
- Schema: `meeting_speakers(id, meeting_id, label, member_id NULL, centroid BLOB)` e
  `ALTER TABLE transcripts ADD COLUMN speaker_id TEXT`. Campo `speaker` ('mic'/'system') permanece.
- Fase 2 funciona sem diarização (speaker_ids vazio); esta fase só enriquece metadados.

### Fase 4 — Entidades e fatos (tickets, decisões, ações)

- Na indexação, 1 chamada LLM por reunião (provider de resumo já configurado, via `summary/llm_client.rs`)
  com saída JSON: tickets mencionados (normalizados pelos `ticket_patterns` + regex antes do LLM), status,
  bloqueios e motivo, responsáveis, decisões, action items — cada fato com `chunk_id`/timestamp de evidência.
- Tabelas: `entities(id, project_id, type['ticket'|'person'|'topic'], key, display_name, source['meeting'|'jira'…])`
  e `entity_facts(id, entity_id, meeting_id, chunk_id, fact_type['status'|'blocker'|'decision'|'action'],
  content, speaker_id, occurred_at)`.
- Coluna `source` já prepara integrações futuras (Jira/Linear virariam outro "ingestor" alimentando as mesmas
  tabelas + uma ferramenta a mais no planner) — sem implementar agora.
- UI: aba "Tickets" no projeto com linha do tempo de cada ticket (fatos + link para o trecho).

### Fase 5 — Consulta: "Pergunte ao projeto"

- `rag/planner.rs`: 1 chamada LLM produz JSON `{intent, rewritten_query, date_range, meeting_hint,
  speaker_hint, entity_keys, tools}`; datas relativas ("semana passada") resolvidas com a data atual no prompt;
  nomes resolvidos contra `project_members`. Fallback sem planner: hybrid search puro.
- `rag/retriever.rs`: executa ferramentas, top-k (≈20 vector + 20 BM25) → RRF (k=60) → top 8–12;
  para `entity_lookup` retorna fatos ordenados por data (o **mais recente** responde "está bloqueado por quê").
- Grade de evidência: limiar de score + checagem LLM curta; insuficiente → 1 retry com query reescrita, depois
  resposta explícita "não encontrei nas reuniões deste projeto".
- Resposta via provider LLM configurado, streaming por eventos Tauri (padrão já usado no summary), com
  citações estruturadas `{meeting_id, title, date, speaker, start_time}`.
- Frontend: página `src/app/projects/[id]/ask` (ou painel na Sidebar) com chat, histórico por projeto
  (`rag_conversations` opcional), e citação clicável que abre `meeting-details` no timestamp
  (`AudioPlayer.tsx` já suporta sync por tempo).
- Busca existente (`api_search_transcripts`) passa a usar FTS5 + escopo de projeto.

---

## Arquivos críticos

- `frontend/src-tauri/migrations/` — novas migrations (projects, rag_index, diarization, entities, settings).
- `frontend/src-tauri/src/database/{models.rs, repositories/*.rs}` — novos repositórios; `meeting.rs`/`transcript.rs` com `project_id`.
- `frontend/src-tauri/src/api/api.rs` — `api_save_transcript` com `project_id` + gatilho de indexação.
- `frontend/src-tauri/src/lib.rs` — registro de comandos.
- `frontend/src-tauri/src/summary/{llm_client.rs, service.rs}` — reuso do cliente LLM; contexto do projeto no prompt.
- `frontend/src-tauri/src/ollama/ollama.rs` — reuso de endpoint/pull de modelo.
- Novos: `src-tauri/src/{projects, rag, diarization}/`.
- Frontend: `src/contexts/ProjectContext.tsx`, `components/Sidebar/*`, `src/app/projects/**`, `app/meeting-details` (falantes + deep link), settings de embedding/diarização.

## Status

- [x] Fase 1 — Projetos (PR #1)
- [x] Fase 2 — Indexação (`src-tauri/src/rag/`): chunking por janela de tempo, embeddings via Ollama,
  FTS5, busca híbrida com RRF, reindexação por projeto, aba *Knowledge* nas configurações e painel de
  busca na página do projeto (PR #1)
- [x] Fase 3 — Diarização (`src-tauri/src/diarization/`): segmentação pyannote 3.0 + embeddings CAM++
  (ONNX, modelos baixados sob demanda), clustering aglomerativo, atribuição por sobreposição aos trechos
  da transcrição, reconhecimento de membros por voiceprint aprendido ao vincular falantes; nomes aparecem
  na transcrição e nos trechos indexados ("Ana: …")
- [x] Fase 4 — Entidades e fatos (`rag/entities.rs`): após indexar, o LLM de resumo extrai status, bloqueios,
  decisões e ações (com o trecho de origem); tickets reconhecidos pelos padrões do projeto; fatos entram como
  evidência prioritária no *Ask* e aparecem nas abas Tickets / Decisions / Action items do projeto
- [x] Fase 5 — Perguntas e respostas com citações (`rag/answer.rs`, página *Ask*): planner via LLM
  (consulta + filtros de data/reunião), busca híbrida com relaxamento de filtros, bloqueio sem evidência
  e resposta citando trechos [n] com reunião, data e minuto. Ainda sem streaming, sem histórico
  persistido e sem abrir o áudio no minuto citado.

## Ordem de entrega sugerida

1. Fase 1 (Projetos) — valor imediato e base de escopo.
2. Fase 2 + Fase 5 mínima (hybrid search + resposta com citações, sem planner) — primeiro "pergunte ao projeto".
3. Planner + grade de evidência.
4. Fase 4 (tickets/fatos).
5. Fase 3 (diarização) — maior risco técnico; o resto não depende dela.

## Verificação

- `cargo check` / `cargo test` em `frontend/src-tauri`; testes unitários para: chunker (janelas, overlap,
  quebra por falante), cosine/RRF, parser do JSON do planner e das entidades, migrations aplicando sobre DB
  existente (backfill do projeto "Geral").
- `pnpm run build` / lint no frontend.
- E2E manual (`./clean_run.sh`, Ollama rodando com `bge-m3`): criar 2 projetos; importar/gravar 3 reuniões
  (uma mencionando "ABC-123 bloqueado por falta de acesso ao ambiente"); confirmar indexação via eventos/logs
  (`RUST_LOG=app_lib::rag=debug`); perguntar:
  - "o que foi falado na reunião de <data>?" → resumo + citações corretas;
  - "ABC-123 está bloqueado por quê?" → motivo + reunião/data;
  - "em que dia <membro> falou sobre deploy?" → após diarização e atribuição do falante;
  - mesma pergunta no outro projeto → não vaza resultados entre projetos;
  - pergunta sem resposta → "não encontrei".
- Desligar Ollama durante indexação → job fica pendente e é retomado.

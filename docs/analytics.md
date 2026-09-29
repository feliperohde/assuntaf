# Estatísticas de uso (PostHog e Google Analytics 4)

As estatísticas só são enviadas depois que o usuário ativa **Configurações → Geral → Estatísticas de uso**.
Cada serviço só é usado se a sua chave estiver preenchida; com as duas, cada evento vai para os dois.
Títulos, caminhos de arquivo e nomes de dispositivos são removidos antes do envio; transcrições e
resumos nunca são enviados.

As chaves ficam em `frontend/src-tauri/src/analytics/commands.rs`:

```rust
const POSTHOG_API_KEY: &str = "";                        // phc_…
const POSTHOG_HOST: &str = "https://us.i.posthog.com";   // ou https://eu.i.posthog.com
const GA_MEASUREMENT_ID: &str = "";                      // G-…
const GA_API_SECRET: &str = "";
```

Também podem vir de variáveis de ambiente no momento do build (têm prioridade sobre as constantes):
`ASSUNTA_POSTHOG_KEY`, `ASSUNTA_POSTHOG_HOST`, `ASSUNTA_GA_MEASUREMENT_ID`, `ASSUNTA_GA_API_SECRET`.

## PostHog

1. Crie uma conta em <https://posthog.com> (escolha a região **US** ou **EU**).
2. Crie uma organização e um projeto (ex.: "Assunta").
3. Em **Project settings → General**, copie o **Project API key** (começa com `phc_`).
4. Cole em `POSTHOG_API_KEY`. Se o projeto for da região EU, troque `POSTHOG_HOST` para
   `https://eu.i.posthog.com` (ou o endereço do seu PostHog auto-hospedado).
5. Os eventos aparecem em **Activity → Events** poucos segundos depois.

A chave `phc_` é pública por natureza (só permite enviar eventos), então pode ficar no código.

## Google Analytics 4

O app envia pelo **Measurement Protocol** do GA4, direto do núcleo em Rust.

1. Acesse <https://analytics.google.com> e clique em **Admin** (engrenagem).
2. **Create → Property**: nome "Assunta", fuso e moeda; avance pelas perguntas do negócio e crie.
3. Em **Data collection**, escolha a plataforma **Web** (o Measurement Protocol usa um stream Web;
   o de app exige Firebase). Informe qualquer URL (ex.: `https://github.com/feliperohde/assunta`),
   um nome para o stream e clique em **Create stream**.
4. Na tela do stream, copie o **Measurement ID** (`G-XXXXXXXXXX`) → `GA_MEASUREMENT_ID`.
5. Ainda no stream, abra **Measurement Protocol API secrets** → **Create**, dê um apelido e copie o
   **Secret value** → `GA_API_SECRET`.
6. Os eventos aparecem em **Reports → Realtime** em até um minuto (relatórios completos levam até 24–48 h).

Para validar um evento sem registrá-lo, envie o mesmo corpo para
`https://www.google-analytics.com/debug/mp/collect?measurement_id=…&api_secret=…`, que responde
com os erros de validação.

O API secret só permite enviar eventos para esse stream; mesmo assim, se o repositório for público,
prefira as variáveis de ambiente no build em vez de gravar o secret no código.

## O que é enviado

Nomes de eventos como `app_started`, `session_started`, `recording_started`, `recording_stopped`,
`page_view_<página>`, `button_click_<botão>`, `summary_generation_completed` e `error`, com parâmetros
como versão do app, duração, provedor/modelo e plataforma.

No GA4:
- os nomes são normalizados (letras, números e `_`, até 40 caracteres), com no máximo 25 parâmetros
  por evento;
- números viram métricas;
- o ID anônimo do usuário é o `client_id`;
- a sessão do app vira o `session_id`.

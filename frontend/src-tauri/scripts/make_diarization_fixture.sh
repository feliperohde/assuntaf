#!/usr/bin/env bash
# Builds a synthetic two-voice Portuguese dialogue (16 kHz mono WAV) for the
# diarization end-to-end test. Needs espeak-ng and ffmpeg.
#
#   ./scripts/make_diarization_fixture.sh /tmp/diar/dialogue.wav
#   DIARIZATION_MODELS_DIR=/path/to/models DIARIZATION_TEST_WAV=/tmp/diar/dialogue.wav \
#     cargo test --lib diarization -- --nocapture
#
# Models: segmentation-3.0.onnx and wespeaker_en_voxceleb_CAM++.onnx from
# https://github.com/thewh1teagle/pyannote-rs/releases/tag/v0.1.0
set -euo pipefail
out="${1:?usage: $0 output.wav}"
ffmpeg_bin="${FFMPEG:-ffmpeg}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

lines=(
  "pt-br+m3|Bom dia pessoal, vamos começar a reunião de hoje sobre o projeto de pagamentos."
  "pt-br+f4|Bom dia. O ticket A B C cento e vinte e três continua bloqueado porque ainda não temos acesso ao ambiente de homologação."
  "pt-br+m3|Entendi. Quem pode pedir esse acesso para o time de infraestrutura ainda hoje?"
  "pt-br+f4|Eu posso abrir o chamado hoje à tarde e amanhã de manhã eu falo com o time de infraestrutura."
  "pt-br+m3|Perfeito. Então a gente decide adiar o deploy para sexta feira, combinado?"
  "pt-br+f4|Combinado. Vamos mover a entrega para sexta e revisar o status de novo na quinta."
)

: > "$work/list.txt"
i=0
for line in "${lines[@]}"; do
  i=$((i + 1))
  espeak-ng -v "${line%%|*}" -s 150 -w "$work/raw$i.wav" "${line#*|}"
  "$ffmpeg_bin" -loglevel error -y -i "$work/raw$i.wav" -ar 16000 -ac 1 "$work/p$i.wav"
  "$ffmpeg_bin" -loglevel error -y -f lavfi -i anullsrc=r=16000:cl=mono -t 0.6 "$work/s$i.wav"
  printf "file '%s'\nfile '%s'\n" "$work/p$i.wav" "$work/s$i.wav" >> "$work/list.txt"
done
"$ffmpeg_bin" -loglevel error -y -f concat -safe 0 -i "$work/list.txt" -ar 16000 -ac 1 -c:a pcm_s16le "$out"
echo "wrote $out"

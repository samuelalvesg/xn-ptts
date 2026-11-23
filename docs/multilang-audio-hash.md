# Hash de áudio de referência (ptts-ws-server)

`the_default_path_still_produces_the_upstream_audio` compara o FNV-1a do PCM s16le gerado
sem `lang` no setup (`--lang en`, voz padrão, seed 7, q8, CPU, texto fixo) com o valor
gerado pelo upstream `c81eca4`.

Para regerar: criar um worktree de `c81eca4`, copiar o corpo do teste (com a assinatura de
`load_ptts` do upstream), rodar com
`PTTS_TEST_MODEL=hf://gradium/phonon-40m-multilingual cargo test -p ptts-ws-server -- --ignored --nocapture`
e ler a linha `fnv1a`. Nunca regerar a partir da branch `multilang`.

O hash depende de CPU e versão do kernel de q8: se mudar de máquina, regere no upstream da mesma máquina.

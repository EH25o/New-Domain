# ffrs: base para reescrever o Final Fight (SNES) em Rust

Ferramentas de engenharia reversa e mapa de memória do **Final Fight (USA)**, sem dependências externas.
A ROM **não** vem junto e nunca é embutida: o programa lê a sua própria cópia do disco.
Isto é a fundação da reescrita, **não** o jogo pronto.

## Como usar

```bash
cargo run --release -- info   Final_Fight_USA.sfc
cargo run --release -- disasm Final_Fight_USA.sfc 80:832F --fn --dp0   # NMI inteiro, com nomes
cargo run --release -- disasm Final_Fight_USA.sfc 02:81B7 60 --m16
cargo run --release -- wramdiff wram_parado.bin wram_direita.bin 0000-1FFF
cargo run --release -- oam wram_parado.bin
cargo run --release -- map
cargo run --release -- cfg    Final_Fight_USA.sfc --tables tables/final_fight_usa.txt --csv out --ghidra ff_ghidra.py
cargo run --release -- boot   Final_Fight_USA.sfc 160 --dump wram_emulada.bin   # roda no interpretador
cargo run --release -- boot   Final_Fight_USA.sfc 900                       # SPC700 executa o driver enviado
cargo test                                       # testes sem ROM
FF_ROM=Final_Fight_USA.sfc cargo test --release  # inclui o teste de regressão contra a WRAM real (PowerShell: $env:FF_ROM=...)
# Opcional: SST_65816=<pasta do repositório SingleStepTests/65816> cargo test --release
```

O executor de conformidade do 65816 lê os vetores JSON da pasta indicada por `SST_65816` (ou sua subpasta `v1`),
sem copiar a suíte para o projeto nem adicionar dependências. A suíte pública está em
[SingleStepTests/65816](https://github.com/SingleStepTests/65816). Em PowerShell:

```powershell
$env:FF_ROM = "Final_Fight_USA.sfc"
$env:SST_65816 = "C:\caminho\SingleStepTests\65816"
cargo test --release
```

Endereços são `BANCO:ENDEREÇO` em hex. Os bancos `80+` são espelho dos bancos `00+` (LoROM).
Use a metade alta do banco (`$8000-$FFFF`) para ler a ROM.

## Arquivos

| Arquivo | Função |
|---|---|
| `src/rom.rs` | Carrega a ROM, valida cabeçalho/checksum, mapeia LoROM, lê vetores |
| `src/disasm.rs` | Desassemblador 65C816 com rastreamento dos flags M/X e anotação de endereços |
| `src/optable.rs` | Tabela dos 256 opcodes (gerada) |
| `src/cpu.rs`, `bus.rs`, `ppu.rs`, `apu.rs`, `boot.rs` | **Interpretador 65816 completo** + barramento/DMA + estado da PPU + protocolo de upload da APU, e o laço de quadros (`boot`) |
| `src/spc700.rs`, `src/dsp.rs` | Interpretador SPC700 validado por vetores SST; DSP fase A com registradores/vozes e relógio de amostragem, sem áudio |
| `src/cfg.rs` | **Descoberta de código** (estágio 1 da recompilação): segue desvios/chamadas/tabelas e separa código de dados |
| `tables/final_fight_usa.txt` | Tabelas de salto conferidas à mão (só endereços) |
| `src/wram.rs` | **Mapa de memória** com nível de confiança + decodificador de OAM. Edite aqui |
| `src/main.rs` | Linha de comando |

Limitação do desassemblador: varredura linear. Ele segue `REP`/`SEP`, mas não segue desvios,
então depois de um salto use `--m16`/`--x16` se o trecho começar em 16 bits.

## O que já foi descoberto

**ROM:** LoROM 1 MB, sem coprocessador nem SRAM, checksum `B6D4`.
Vetores: RESET `00:8000`, NMI `00:832F`, IRQ `00:87DF`.

**Estrutura de cada quadro (NMI em `80:832F`)**, lida do código:

1. Salva registradores, `D=0`, incrementa `$C9` (contador de quadros) e `$C8`.
2. Se `$00CE != 0`, chama em ordem: `$87BE`, `$86A8`, `$85C8`, `$8529`; se `$D7 == 0`, também
   `$8657`, `$8714`, `$8741`, `$84EB`, `$83C4`; copia `$E8` para `$2100`.
3. Chama `$8406` e percorre a `task_table` (`$40` a `$C0`, passo `$10`): se o byte `+0` vale 1,
   decrementa `+1` (espera) e, ao zerar, volta `+0` para 4.
4. Chama `$8852`, religa o NMI (`JSL $0088B0`) e retorna.

**Kernel de tarefas (confirmado pelo código):** o despachante em `00:814E` faz `LDA $0042,Y` -> `TCS` (troca a pilha
da CPU pelo SP salvo da tarefa) e `LDX $0046,Y` -> `JMP ($8237,X)`, uma tabela de 12 trampolins `JML` em `00:824F-827B`
que levam às rotinas de cada estado (bancos 00 e 01). Ou seja, o jogo é um escalonador cooperativo com uma pilha por
tarefa, e cada tarefa na `task_table` tem `+2` = SP salvo, `+6` = índice do estado. Isso importa para a reescrita:
cada tarefa vira uma corrotina (ou um autômato de estados explícito) em Rust, não um fluxo único.

**Sprites:** o buffer OAM fica em `$0180-$037F` (128 x 4 bytes: X, Y, tile, atributos) e a tabela
alta em `$0380-$039F`. A rotina `01:A847` o limpa a cada quadro (Y=`$E0`). A montagem dos sprites
está em `02:8000` -> `02:80A0` -> `02:8161` -> `02:81B7`. Em `02:81B7` o `D` passa a apontar para o objeto
(`PHD`/`PHY`/`PLD`), e a rotina subtrai de seus campos `+$07` e `+$0A` os valores `$0E07` e `$0E0A`
(provavelmente a câmera). Depois ela carrega o ponteiro `+$1C` do objeto (provavelmente a definição do sprite)
e salta por uma tabela de pulos indexada pelo byte `+4` dessa estrutura.

Use `cargo run -- map` para ver todos os endereços com o nível de confiança (confirmado ou hipótese).

## Recompilação estática: onde estamos

| Estágio | Estado |
|---|---|
| 1. Descobrir todo o código alcançável (`cfg`) | **feito**, com a ressalva abaixo |
| 2. Interpretador 65816 de referência, para teste diferencial | **funciona** (veja abaixo) |
| 3. Gerador de código Rust a partir do grafo | não iniciado |
| 4. Runtime: PPU (desenho), SPC700/DSP (som) e entrada | parcial: SPC700 executa o driver; DSP fase A sem áudio; PPU sem desenho |
| 5. Validação quadro a quadro contra o Mesen2 | parcial (teste de regressão abaixo) |

**Estado atual do boot.** `boot` roda a ROM real do reset, faz o upload do driver de som (5 blocos), inicia o SPC700 no
endereço enviado e chega a 900 quadros sem o PC parado. `$C9` continua avançando depois do quadro 190. O SPC700 foi
comparado com os vetores SingleStepTests; a IPL segue em HLE e o DSP fase A ainda não gera áudio.
O teste `boots_and_matches_real_wram_structure` compara o resultado com dumps reais do Mesen2: `$E8=0F`, `$0108=31`,
`$CE=01` e os **8 topos de pilha da `task_table`** (`$1DFF, $1E3F, $1E7F, $1EBF, $1EFF, $1F3F, $1F7F, $1FBF`) batem.
Só os ponteiros de pilha salvos (`+2`) diferem, como esperado, porque os estados do jogo são outros. O novo teste
`boots_through_900_frames_with_the_uploaded_spc_driver` verifica que `$C9` muda após o quadro 190 e que a CPU não para.

O handshake de "pronto" simulado foi removido. A APU mantém a IPL em HLE durante o upload e, ao receber o comando de
execução, transfere a RAM e inicia o SPC700 real. A reentrada na IPL em `$FFC0` é detectada e reinicia o handshake HLE;
ela ainda não foi observada na execução de 900 quadros.
O DSP não decodifica BRR, então a execução prolongada confirma o avanço do jogo, mas não som correto.

**O que o estágio 1 mostrou (ROM do Final Fight USA):**
* O código ocupa só **~115 KB (11% da ROM), em 7 dos 32 bancos (00 a 06)**; o resto é gráficos, música e dados de fase.
* Há **~2.200 funções** e **662 saltos indiretos** por tabela. Com `--speculative` a heurística resolve 658 deles; só 4 apontam
  para ponteiros em RAM e precisam de análise manual.
* Qualidade com heurística: 17 instruções sobrepostas e 7 endereços ambíguos em ~51 mil instruções (0,03%). Esses são os pontos a revisar.
* Sem heurística, a análise para cedo: cada tabela resolvida à mão libera novos despachantes (3 tabelas -> 13 pendentes).
  O jogo é um escalonador de tarefas, então quase todo o código está atrás de `JMP (tabela,X)`.

`--ghidra` gera um script que desmonta cada função com o estado M/X certo, o que contorna o problema de a análise
automática do Ghidra não achar o código sozinha. Ele **não foi testado** num Ghidra real; use Ctrl+Z se algo sair errado.

## O que ainda NÃO está mapeado

Posição X/Y/Z do jogador no mundo, vida, lista de inimigos, golpes e hitboxes, formato dos dados de
fase e tiles. A pista mais forte é a rotina `02:81B7`: cada objeto é uma estrutura apontada pelo
Direct Page, com campos de posição em `+$07`/`+$0A`. O próximo passo é descobrir quem chama essa rotina
com cada objeto em `Y` (o laço que percorre a lista de objetos, nas chamadas `02:8000`/`02:80A0`).

## Roteiro para a reescrita (método usado por reescritas como a do `zelda3`)

1. Mapear a estrutura de objeto e as rotinas do quadro acima, uma por vez.
2. Reescrever cada rotina como função Rust sobre um `struct State` que espelha a WRAM.
3. Validar contra o emulador: gravar a WRAM a cada quadro no Mesen2 e comparar byte a byte com a sua
   versão em Rust. Divergiu, a rotina está errada.
4. Só depois de a lógica bater, trocar a saída de vídeo (OAM/VRAM) por um renderizador em Bevy,
   carregando os tiles e paletas da sua ROM em tempo de execução.
5. Terry e os outros lutadores do KOF XIII entram como dados externos (golpes, hitboxes) lidos dos
   seus arquivos, sem embutir ativos no repositório.

## Aviso

Use apenas cópias que você possui. Não distribua a ROM nem sprites/dados extraídos; só o código.

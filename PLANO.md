# PrimoDock Linux — plano técnico

Porte nativo do PrimoDock (macOS) para Linux, alvo inicial: a máquina do autor.
Restrição dura: **nativo, nada de Electron ou webview**.

---

## 1. Diagnóstico da máquina alvo

| Item | Valor | Implicação |
|---|---|---|
| Distro | Ubuntu 22.04.5 LTS | Suporte padrão até abr/2027 |
| Shell | GNOME Shell 42.9 / Mutter 42.9 | API de extensão da era GNOME 42 |
| Sessão | **X11** (`ubuntu-xorg`) | EWMH disponível, controle total de janelas |
| GPU | AMD Lucienne (iGPU Ryzen) | **Sem impedimento para Wayland** |
| Sessão Wayland | Instalada e disponível | X11 é escolha, não necessidade |
| Dock atual | **Plank** (`ubuntu-dock` desativado) | Prova viva de que GTK3+EWMH+strut funciona aqui |
| Monitores | 1× eDP 1920×1080 | Multi-monitor sai do escopo inicial |
| Workspaces | 4, via `_NET_NUMBER_OF_DESKTOPS` | Base pronta para "ambientes" |
| Toolchain | gcc 11, python3, node 24 | Falta Rust, pkg-config, libs de dev |

---

## 2. A tensão central

O requisito "sem gambiarra" e a escolha por GNOME colidem, e é melhor
encarar isso antes de escrever a primeira linha.

**No X11**, um dock externo não é gambiarra nenhuma — é a arquitetura
historicamente correta. `_NET_WM_STRUT_PARTIAL` reserva o espaço na borda,
`_NET_CLIENT_LIST` lista janelas, `_NET_ACTIVE_WINDOW` e `_NET_WM_STATE`
controlam. Tudo protocolo público e documentado. Plank, Docky e Cairo-Dock
fazem exatamente isso há quinze anos. Você escreve 100% do app no toolkit
que escolher, e zero JavaScript.

**No GNOME Wayland**, isso é impossível. O Mutter não expõe `wlr-layer-shell`
(não há como ancorar uma barra) nem gerenciamento de toplevel de terceiros
(não há como esconder a janela dos outros). O único ponto de extensão
sancionado é a extensão de Shell em GJS, rodando dentro do processo do Mutter.

E o detalhe que dói: **o GNOME upstream removeu a sessão X11.** Construir
X11-only em 2026 é escolher voluntariamente uma plataforma com data de
validade, estando você em hardware AMD onde o Wayland funciona bem.

### As três arquiteturas, sem eufemismo

| Arquitetura | É gambiarra? | Custo |
|---|---|---|
| **A. App externo X11 (EWMH)** | Não. Protocolo público. | Morre junto com o X11. Sua próxima atualização de Ubuntu já é o prazo. |
| **B. Extensão de Shell (GJS)** | Não *se* usar API pública. Vira gambiarra se fizer monkey-patch em interno do Shell, como o dash-to-dock faz. | UI obrigatoriamente em JavaScript/Clutter. Quebra a cada versão do GNOME. |
| **C. Daemon nativo + extensão fina, via DBus** | Não. | Mais peças, mas é a única que sobrevive à travessia para o Wayland. |

---

## 3. O que "nativo" custa em cada caminho

Essa é a decisão real, e ela não é sobre linguagem — é sobre **quem desenha
a barra**.

**Caminho X11:** você desenha. GTK ou Qt, seu código, seu toolkit, sua
animação. Zero GJS.

**Caminho GNOME Wayland:** o Mutter desenha. A superfície do dock tem que ser
um ator Clutter/St dentro do Shell, o que significa que *a camada visual
inteira* — os oito temas, a lente de ampliação, a animação de abertura, os
tiles dos widgets — vive em GJS. Não há como escapar disso e continuar no
GNOME.

Vale registrar que GJS/Clutter **satisfaz o requisito**: é a cena gráfica em C
do próprio GNOME Shell, com bindings JS. Não tem Chromium, não tem webview,
não é Electron. Mas é JavaScript, e é bom saber disso antes e não depois.

---

## 4. Arquitetura recomendada: C (híbrida)

Mesmo começando em X11, vale separar desde o dia um:

```
┌─────────────────────────────────────────────┐
│  primodock-shell   (a camada que desenha)   │
│  ─ X11:     GTK3, processo próprio          │
│  ─ Wayland: extensão GJS/St no Shell        │
│  Ícones · temas · ampliação · tiles         │
└──────────────────┬──────────────────────────┘
                   │  DBus  (o contrato)
┌──────────────────┴──────────────────────────┐
│  primodockd   (o cérebro, nativo, portátil) │
│  Ambientes · perfis · config · persistência │
│  Fontes de dados dos widgets · temas        │
│  Índice de .desktop · tradução de janelas   │
└──────────────────┬──────────────────────────┘
                   │
      ┌────────────┴────────────┐
      │  backend de janelas     │
      │  trait WindowManager    │
      │  ├─ X11Backend (EWMH)   │  ← agora
      │  └─ ShellBackend (DBus) │  ← depois
      └─────────────────────────┘
```

O ponto da separação: quando o X11 acabar, você reescreve a camada de cima
e **preserva o daemon inteiro** — que é onde estão os trinta widgets, a
configuração, os ambientes e a persistência. Ou seja, 70% do trabalho.

Sem essa fronteira, migrar depois significa reescrever tudo.

### Contrato DBus (`dev.oprimo.PrimoDock`)

```
Métodos
  ListItems()              → a[(id, type, payload)]   itens do dock atual
  Activate(id)             → ()                        clique
  ActivateWindow(wid)      → ()
  SetEnvironment(name)     → ()
  ListEnvironments()       → as
  GetWidgetState(id)       → a{sv}                     estado do tile
  InvokeWidget(id, action) → a{sv}

Sinais
  ItemsChanged()
  WidgetStateChanged(id, a{sv})     ← alta frequência, ver §8
  EnvironmentChanged(name)
  WindowsChanged()
```

O daemon nunca sabe como o item é desenhado; a casca nunca sabe de onde o
dado veio. É essa ignorância mútua que torna a migração barata.

---

## 5. Stack concreta

### Daemon
**Rust.** Ecossistema bom para tudo que o daemon precisa: `zbus` (DBus),
`x11rb` (EWMH sem FFI insegura), `tokio`, `serde`, `reqwest`. Binário único,
sem runtime.

### Casca X11 — **GTK3, não GTK4**

Isso é contraintuitivo e importa. O GTK4 **removeu** as ferramentas que um
dock X11 precisa: não há mais `GDK_WINDOW_TYPE_HINT_DOCK`, e as escotilhas
para falar X11 direto foram fechadas. Colocar strut num `GtkWindow` do GTK4
exige pescar o XID e fazer `XChangeProperty` na mão, contra a corrente do
toolkit.

O GTK3 tem tudo pronto e testado em batalha — é o que o Plank usa. Está em
manutenção, não em abandono, e para uma ferramenta pessoal com horizonte de
poucos anos isso é aceitável. E há um bônus: o GTK4 disponível no 22.04 é o
**4.6**, de 2022, já bem atrás do upstream.

### Dependências a instalar

```bash
sudo apt install -y build-essential pkg-config libgtk-3-dev \
  libwnck-3-dev libxcb-ewmh-dev libcairo2-dev libpango1.0-dev \
  libglib2.0-dev meson ninja-build
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

---

## 6. Ambientes: a parte que fica *mais fácil* que no macOS

No macOS, "ambiente" exige esconder janela à força. Aqui os workspaces já
existem de verdade — você tem 4 configurados agora.

O modelo: **ambiente = perfil de dock + conjunto de workspaces**.

```
Ambiente "Trabalho"   → workspaces {0,1}, perfil de dock A
Ambiente "Estudos"    → workspace  {2},   perfil de dock B
Ambiente "Pessoal"    → workspace  {3},   perfil de dock C
```

Trocar de ambiente é `_NET_CURRENT_DESKTOP` + trocar o perfil. As janelas
dos outros ambientes saem da frente **porque o WM já faz isso** — não porque
você escondeu nada. É mais robusto que o original, não menos.

O atalho de teclado não é capturado pelo app. `CycleEnvironment` e
`SetEnvironment` são métodos D-Bus, e uma custom keybinding do GNOME chama um
deles — ver o README.

Substituto do "Segue o seu Foco": não existe Focus Mode no GNOME. O análogo
honesto é trocar por horário, por rede Wi-Fi conectada, ou por atalho. Não
tente emular o Foco da Apple.

---

## 7. Triagem dos widgets para este setup

**Portam direto (dados do sistema, sem atrito):**
relógio, cronômetro, timer, pomodoro, contagem regressiva, progresso do
tempo, nota, conversor, beber água, emoji, favoritos, coleções de apps,
mascote. Bateria via UPower (DBus), CPU/memória via `/proc`, rede via
`/proc/net/dev`, clima e GitHub e uso do Claude via HTTP.

**Ficam melhores que no macOS:**
- **Música** — MPRIS por DBus é um protocolo único que cobre Spotify, VLC,
  mpv e navegador de uma vez. No macOS o app precisa de integração por app.

**Precisam de substituto:**
- Calendário e Lembretes → Evolution Data Server via DBus
- Capturas de tela e conta-gotas → portal XDG (`org.freedesktop.portal.Screenshot`)
- Arquivos / Downloads → `inotify`
- Atalhos → não existe app Atalhos; vira disparo de `.desktop` ou script
- Área de transferência → no X11, fácil. **Você já tem o `clipboard-indicator`
  resolvendo isso** — não reimplemente na primeira rodada.

**Cortar do escopo:**
- **AirDrop** — não tem equivalente. O análogo cultural é LocalSend ou
  KDE Connect, mas embutir isso é um segundo produto dentro do primeiro.
- **Prateleira e Drop Zone** — dependem de um modelo de arrasto longo que
  funciona, mas rende pouco antes do resto estar de pé.
- **Multi-monitor** — você tem uma tela. Não projete para o problema que
  não tem.
- **Prévias de janela** — no X11 exige composite redirect e captura de
  pixmap por janela. Caro, e é polimento.

---

## 8. Riscos reais

**Repintura a 60fps mata bateria.** Você está num laptop com iGPU. Um dock
com mascote animado, rede ao vivo e anel de timer pode consumir CPU o dia
inteiro. Regra desde o começo: o daemon empurra `WidgetStateChanged` com
*coalescência* e taxa por widget (rede 1Hz, bateria 30s, clima 15min), e a
casca **para de animar quando o dock está oculto**. Isso é arquitetura, não
otimização posterior — refazer depois é caro.

**GNOME 42 é antigo.** Se você for pelo caminho B ou C, a extensão é escrita
contra uma API de 2022. Sua próxima atualização de Ubuntu vai quebrá-la.
É o argumento mais forte para manter tudo que der fora do GJS.

**X11 tem prazo.** Não é "se", é "quando". O plano acima trata isso
explicitamente pela fronteira DBus, mas vale decidir de olhos abertos: esta
é uma ferramenta pessoal com horizonte de alguns anos, não um produto.

---

## 9. Ordem de implementação

**Fase 0 — Espinha dorsal.** Prove a parte difícil antes de investir em
qualquer beleza: uma barra GTK3 vazia, ancorada com strut, que o GNOME
respeita; daemon em Rust listando janelas por EWMH; os dois conversando por
DBus. Se isso não ficar sólido, nada acima dele fica.

**Fase 1 — Dock utilizável.** Apps fixados lidos de `.desktop`, indicador de
janela aberta, clique ativa/minimiza, botão direito com menu. Neste ponto já
substitui o dock que você não tem.

**Fase 2 — Ambientes.** Perfis, mapeamento para workspaces, troca por clique
no chip ou por atalho do próprio desktop.

> **Correção (2026-09-29):** este plano assumia o portal `GlobalShortcuts`
> para o atalho de teclado. Ele não existe aqui — o `xdg-desktop-portal` do
> Ubuntu 22.04 é o 1.14.4 e essa interface só entrou na 1.17. O caminho certo
> no Linux é outro e é melhor: o app **expõe a ação** por D-Bus e o desktop
> liga a tecla nela, via custom keybinding do GNOME. Quem é dono do teclado é
> o ambiente, não o app.

**Fase 3 — Infra de widget + cinco.** O contrato de tile e o painel. Comece
por relógio, bateria, CPU, música (MPRIS) e pomodoro — cobrem os quatro
formatos de tile e provam a arquitetura de atualização.

**Fase 4 — Temas.** CSS do GTK3 já dá quase tudo. Dois temas, não oito.

**Fase 5 — Paridade e o resto dos widgets.** Só aqui: pilhas, ampliação,
lixeira, arrastar e soltar.

---

## 10. A decisão que falta

Três caminhos, e eles não se equivalem:

1. **X11 puro, assumindo o prazo.** Mais rápido, 100% no seu toolkit, zero
   JavaScript. Ferramenta pessoal boa por uns anos.
2. **Híbrido com fronteira DBus, começando em X11.** Mesmo começo, custo
   extra pequeno agora, migração barata depois. É o que este plano descreve.
3. **Mudar de casa.** Você está em AMD, com Wayland funcionando. Um
   compositor wlroots ou o KDE Plasma dariam layer-shell e gerenciamento de
   toplevel de verdade — a plataforma onde este projeto específico é
   dramaticamente mais fácil. Custa trocar de desktop.

Recomendação: **(2)**. Entrega valor na Fase 1, não desperdiça o trabalho
quando o X11 acabar, e não exige que você troque de desktop hoje.

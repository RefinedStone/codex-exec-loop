/*
 * conversation app slice는 TUI가 대화 화면에서 받는 키 입력과 상태 전환을 담당한다.
 * 현재 공개 표면은 controller에 모여 있으며, 상위 app은 이 모듈을 통해 대화 흐름을 조작한다.
 */
mod controller;

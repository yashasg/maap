# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Independent app developers who add tracking, ship releases, and decide what to improve. They know their app; neither they nor the people they share results with should need to know analytics terminology to understand a report.

## Product Purpose

Maap (माप, “measure” in Hindi) stands for Mobile App Analytics Platform. It helps developers answer three questions:

- **While building:** When I open this screen or perform this action, does Maap record what I intended?
- **After shipping:** What changed in how people use the app compared with before the release?
- **When planning:** Which screens and features do people use, where do they stop, and what should I improve next?

## Positioning

Maap is a self-hosted analytics dashboard built on Umami for iOS, macOS, Android, and websites. An app using a compatible TelemetryDeck SDK can point it at Maap without a separate translation service. Screen navigation becomes a pageview; other signals become named events.

## Product Principles

- Explain each number in terms of what people did. A reader should be able to tell whether “100” means 100 people, 100 sessions, or 100 actions.
- Make comparisons interpretable: name the app, the periods being compared, and any filters that change who or what is counted. A change after a release is not proof the release caused it.
- Keep test activity distinct from real usage. A developer should be able to verify a signal without mistaking their own taps for adoption.

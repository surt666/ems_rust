Feature: Oversigt (Overview) module
  As a user
  I want a dashboard, a statistics page and logs
  So that I get an at-a-glance picture of the selected context

  Background:
    Given I am in the company context "Bolag AB Janus"

  Scenario: Dashboard layout
    When I open Oversigt > Dashboard
    Then the URL is "/App/company/997/overview/dashboard"
    And I see a building-benchmark card with a cost gauge and a CO₂e gauge
    And I see a "Call to action" card summarising the biggest energy wastes
    And I see an "Alarmer" card with alarm-setup and unacknowledged-alarm counters
    And I see a "Forbrugsoverblik" section with a combined card and one card per energy type

  Scenario: Each consumption card is an independent, refreshable widget
    When I view a consumption card on the dashboard
    Then it shows a monthly bar chart comparing the current and previous year
    And it shows current-period, comparison-period and full-prior-year totals with trend indicators
    And it has a measure toggle (cost vs consumption) and a year selector
    When I change the year or measure on that card
    Then only that card reloads its data

  Scenario: Benchmark card reflects the context
    When the context changes
    Then the dashboard widgets recompute for the new context

  Scenario: Statistik shows count cards with breakdown tables
    When I open Oversigt > Statistik
    Then the URL is "/App/company/997/overview/statistics"
    And I see count cards for building elements, users, alarms and meters
    And the meters card breaks down by reading type, energy/resource meter type, other meter types and remote-read acquisition source
    And no charts are rendered on this page

  Scenario: Statistik is read-only and context-scoped
    Then every count and breakdown is scoped to the selected context
    And there are no row actions

  Scenario: Logs is an administrator-scope page
    When I am in a company context and open the Logs entry
    Then it does not resolve to a company page
    And the audit-log surface is only available in administrator (department) scope

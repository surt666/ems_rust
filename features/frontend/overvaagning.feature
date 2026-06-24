Feature: Overvågning (Monitoring) module
  As a user
  I want to see triggered alarms and manage alarm configurations
  So that I can react to consumption anomalies

  Background:
    Given I am in the company context "Bolag AB Janus"

  Scenario: Alarmer lists unacknowledged alarms
    When I open Overvågning > Alarmer
    Then the URL is "/App/company/997/monitoring/monitoring_consumption"
    And I see a header with the alarm count and selected count
    And I see a results grid with columns building, energy form, meter, meter id, tags, name, type, recipients, deviation and extra cost
    And the grid is sorted by extra cost descending by default
    And I can filter via the filter overlay grouped into alarm, building and meter filters

  Scenario: Empty alarm list shows a first-class empty state
    Given the context has no unacknowledged alarms
    Then the grid shows an empty-state message instead of rows

  Scenario: Alarm opsætning manages alarm configurations
    When I open Overvågning > Alarm opsætning
    Then the URL is "/App/company/997/monitoring/monitoring_setup"
    And I see a results grid of configured alarms with columns meter type, meter, name, type and recipients
    And I see a "Opret alarm" action to create a new alarm configuration
    And I can switch the alarm view between Alle, Almindelige and Backoffice
    And the create flow is a server-rendered form (not exercised in this read-only spec)

  Scenario: AI Alarmcenter is feature-gated
    Given the AI alarm feature is not licensed for the context
    Then the "AI Alarmcenter" entry does not navigate to a dedicated page
    And it is gated behind a feature flag in our recreation

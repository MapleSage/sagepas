import {
  CrmContext,
  EmptyState,
  ExtensionPointApiActions,
  Link,
  Text,
} from '@hubspot/ui-extensions';
import { hubspot } from '@hubspot/ui-extensions';

interface CrmExtensionProps {
  context: CrmContext;
  actions: ExtensionPointApiActions<'crm.record.tab'>;
}

hubspot.extend<'crm.record.tab'>(({ context, actions }: CrmExtensionProps) => (
  <TicketServicingCard context={context} actions={actions} />
));

const TicketServicingCard = ({ context }: CrmExtensionProps) => {
  const recordId = context.crm.objectId;
  const portalId = context.portal.id;
  const pasUrl = `https://pas.sagesure.io/policies?hubspotPortalId=${encodeURIComponent(portalId)}&hubspotTicketId=${encodeURIComponent(recordId)}`;

  return (
    <EmptyState
      title="SageSure-US Claims and Policy Servicing"
      layout="vertical"
      imageName="documents"
    >
      <Text>
        Resolve this ticket to its linked native policy and open claims and
        servicing by default. If unlinked, choose the policy and confirm the
        link in SagePAS.
      </Text>
      <Link href={pasUrl}>Resolve ticket policy and open servicing</Link>
    </EmptyState>
  );
};
